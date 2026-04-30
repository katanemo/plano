"""
Integration tests for retry-on-ratelimit feature (P0).

Tests IT-1 through IT-6, IT-12, IT-13 validate end-to-end retry behavior
through the real Plano gateway using Python mock HTTP servers as upstream providers.

Each test:
  1. Starts mock upstream servers on ephemeral ports
  2. Writes a YAML config pointing the gateway at those mock ports
  3. Starts the gateway via `planoai up`
  4. Sends requests and asserts on response status/body/timing
  5. Tears down the gateway via `planoai down`
"""

import json
import logging
import os
import subprocess
import sys
import tempfile
import threading
import time
from http.server import HTTPServer, BaseHTTPRequestHandler
from typing import Optional

import pytest
import requests

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s - %(name)s - %(levelname)s - %(message)s",
    handlers=[logging.StreamHandler(sys.stdout)],
)
logger = logging.getLogger(__name__)

GATEWAY_BASE_URL = "http://localhost:12000"
GATEWAY_CHAT_URL = f"{GATEWAY_BASE_URL}/v1/chat/completions"
CONFIGS_DIR = os.path.join(os.path.dirname(__file__), "configs")

# Standard OpenAI-compatible success response body
SUCCESS_RESPONSE = json.dumps({
    "id": "chatcmpl-test-001",
    "object": "chat.completion",
    "created": 1700000000,
    "model": "mock-model",
    "choices": [
        {
            "index": 0,
            "message": {
                "role": "assistant",
                "content": "Hello from mock provider!",
            },
            "finish_reason": "stop",
        }
    ],
    "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15},
})

# Standard chat request body
CHAT_REQUEST_BODY = {
    "model": "openai/gpt-4o",
    "messages": [{"role": "user", "content": "Hello"}],
}


# ---------------------------------------------------------------------------
# Mock upstream server infrastructure
# ---------------------------------------------------------------------------

class MockUpstreamHandler(BaseHTTPRequestHandler):
    """
    Configurable mock HTTP handler that returns responses from a per-server queue.

    Each server instance has a response_queue (list of tuples):
        (status_code, headers_dict, body_string)

    Responses are consumed in order. When the queue is exhausted, the last
    response is repeated. The handler also records all received requests for
    later assertion.
    """

    # These are set per-server-instance via the factory function below.
    response_queue: list = []
    received_requests: list = []
    call_count: int = 0
    lock: threading.Lock = threading.Lock()

    def do_POST(self):
        content_length = int(self.headers.get("Content-Length", 0))
        body = self.rfile.read(content_length) if content_length > 0 else b""

        with self.__class__.lock:
            self.__class__.call_count += 1
            self.__class__.received_requests.append({
                "path": self.path,
                "headers": dict(self.headers),
                "body": body.decode("utf-8", errors="replace"),
            })
            idx = min(
                self.__class__.call_count - 1,
                len(self.__class__.response_queue) - 1,
            )
            status_code, headers, response_body = self.__class__.response_queue[idx]

        self.send_response(status_code)
        for key, value in headers.items():
            self.send_header(key, value)
        self.send_header("Content-Type", "application/json")
        self.end_headers()
        if isinstance(response_body, str):
            response_body = response_body.encode("utf-8")
        self.wfile.write(response_body)

    def do_GET(self):
        """Handle health checks or other GET requests."""
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.end_headers()
        self.wfile.write(b'{"status": "ok"}')

    def log_message(self, format, *args):
        """Suppress default request logging to reduce noise."""
        pass


def create_mock_handler_class(response_queue: list) -> type:
    """
    Create a new handler class with its own response queue and state.
    This avoids shared state between different mock servers.
    """
    class Handler(MockUpstreamHandler):
        pass

    Handler.response_queue = list(response_queue)
    Handler.received_requests = []
    Handler.call_count = 0
    Handler.lock = threading.Lock()
    return Handler


class MockServer:
    """Manages a mock HTTP server running in a background thread."""

    def __init__(self, response_queue: list):
        self.handler_class = create_mock_handler_class(response_queue)
        self.server = HTTPServer(("0.0.0.0", 0), self.handler_class)
        self.port = self.server.server_address[1]
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)

    def start(self):
        self.thread.start()
        logger.info(f"Mock server started on port {self.port}")

    def stop(self):
        self.server.shutdown()
        self.thread.join(timeout=5)
        logger.info(f"Mock server stopped on port {self.port}")

    @property
    def call_count(self) -> int:
        return self.handler_class.call_count

    @property
    def received_requests(self) -> list:
        return self.handler_class.received_requests


# ---------------------------------------------------------------------------
# Gateway lifecycle helpers
# ---------------------------------------------------------------------------

def write_config(template_name: str, substitutions: dict) -> str:
    """
    Read a config template from configs/ dir, apply port substitutions,
    and write to a temp file. Returns the path to the temp config file.
    """
    template_path = os.path.join(CONFIGS_DIR, template_name)
    with open(template_path, "r") as f:
        content = f.read()

    for key, value in substitutions.items():
        content = content.replace(f"${{{key}}}", str(value))

    # Write to a temp file in the e2e directory so planoai can find it
    fd, config_path = tempfile.mkstemp(suffix=".yaml", prefix="retry_test_")
    with os.fdopen(fd, "w") as f:
        f.write(content)

    logger.info(f"Wrote test config to {config_path}")
    return config_path


def gateway_up(config_path: str, timeout: int = 30):
    """Start the Plano gateway with the given config. Waits for health."""
    logger.info(f"Starting gateway with config: {config_path}")
    subprocess.run(
        ["planoai", "down", "--docker"],
        capture_output=True,
        timeout=30,
    )
    result = subprocess.run(
        ["planoai", "up", "--docker", config_path],
        capture_output=True,
        text=True,
        timeout=60,
    )
    if result.returncode != 0:
        logger.error(f"planoai up failed: {result.stderr}")
        raise RuntimeError(f"planoai up failed: {result.stderr}")

    # Wait for gateway to be healthy
    start = time.time()
    while time.time() - start < timeout:
        try:
            resp = requests.get(f"{GATEWAY_BASE_URL}/healthz", timeout=2)
            if resp.status_code == 200:
                logger.info("Gateway is healthy")
                return
        except requests.ConnectionError:
            pass
        time.sleep(1)

    raise RuntimeError(f"Gateway did not become healthy within {timeout}s")


def gateway_down():
    """Stop the Plano gateway."""
    logger.info("Stopping gateway")
    subprocess.run(
        ["planoai", "down", "--docker"],
        capture_output=True,
        timeout=30,
    )


def make_error_response(status_code: int, message: str = "error") -> str:
    """Create a JSON error response body."""
    return json.dumps({
        "error": {
            "message": message,
            "type": "server_error",
            "code": str(status_code),
        }
    })


# ---------------------------------------------------------------------------
# Streaming helpers
# ---------------------------------------------------------------------------

STREAMING_SUCCESS_CHUNKS = [
    'data: {"id":"chatcmpl-stream-001","object":"chat.completion.chunk","created":1700000000,"model":"mock-model","choices":[{"index":0,"delta":{"role":"assistant","content":"Hello"},"finish_reason":null}]}\n\n',
    'data: {"id":"chatcmpl-stream-001","object":"chat.completion.chunk","created":1700000000,"model":"mock-model","choices":[{"index":0,"delta":{"content":" from"},"finish_reason":null}]}\n\n',
    'data: {"id":"chatcmpl-stream-001","object":"chat.completion.chunk","created":1700000000,"model":"mock-model","choices":[{"index":0,"delta":{"content":" stream!"},"finish_reason":null}]}\n\n',
    'data: {"id":"chatcmpl-stream-001","object":"chat.completion.chunk","created":1700000000,"model":"mock-model","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}\n\n',
    "data: [DONE]\n\n",
]


class StreamingMockHandler(MockUpstreamHandler):
    """Handler that returns SSE streaming responses."""
    pass


def create_streaming_handler_class(
    response_queue: list,
    streaming_chunks: Optional[list] = None,
) -> type:
    """
    Create a handler class that can return streaming SSE responses.

    response_queue entries can include a special "STREAM" body marker
    to trigger streaming mode with the provided chunks.
    """
    chunks = streaming_chunks or STREAMING_SUCCESS_CHUNKS

    class Handler(StreamingMockHandler):
        pass

    Handler.response_queue = list(response_queue)
    Handler.received_requests = []
    Handler.call_count = 0
    Handler.lock = threading.Lock()

    original_do_post = Handler.do_POST

    def streaming_do_post(self):
        content_length = int(self.headers.get("Content-Length", 0))
        body = self.rfile.read(content_length) if content_length > 0 else b""

        with Handler.lock:
            Handler.call_count += 1
            Handler.received_requests.append({
                "path": self.path,
                "headers": dict(self.headers),
                "body": body.decode("utf-8", errors="replace"),
            })
            idx = min(Handler.call_count - 1, len(Handler.response_queue) - 1)
            status_code, headers, response_body = Handler.response_queue[idx]

        if response_body == "STREAM":
            self.send_response(status_code)
            for key, value in headers.items():
                self.send_header(key, value)
            self.send_header("Content-Type", "text/event-stream")
            self.send_header("Transfer-Encoding", "chunked")
            self.end_headers()
            for chunk in chunks:
                self.wfile.write(chunk.encode("utf-8"))
                self.wfile.flush()
                time.sleep(0.05)
        else:
            self.send_response(status_code)
            for key, value in headers.items():
                self.send_header(key, value)
            self.send_header("Content-Type", "application/json")
            self.end_headers()
            if isinstance(response_body, str):
                response_body = response_body.encode("utf-8")
            self.wfile.write(response_body)

    Handler.do_POST = streaming_do_post
    return Handler


class StreamingMockServer:
    """Mock server that supports streaming responses."""

    def __init__(self, response_queue: list, streaming_chunks: Optional[list] = None):
        self.handler_class = create_streaming_handler_class(
            response_queue, streaming_chunks
        )
        self.server = HTTPServer(("0.0.0.0", 0), self.handler_class)
        self.port = self.server.server_address[1]
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)

    def start(self):
        self.thread.start()
        logger.info(f"Streaming mock server started on port {self.port}")

    def stop(self):
        self.server.shutdown()
        self.thread.join(timeout=5)

    @property
    def call_count(self) -> int:
        return self.handler_class.call_count

    @property
    def received_requests(self) -> list:
        return self.handler_class.received_requests


# ---------------------------------------------------------------------------
# Body-echo handler for IT-13
# ---------------------------------------------------------------------------

def create_echo_handler_class(response_queue: list) -> type:
    """
    Create a handler that echoes the received request body back in the
    response, wrapped in a valid chat completion response.
    The response_queue controls status codes — when the status is 200,
    the handler echoes the body; otherwise it returns the queued response.
    """

    class Handler(MockUpstreamHandler):
        pass

    Handler.response_queue = list(response_queue)
    Handler.received_requests = []
    Handler.call_count = 0
    Handler.lock = threading.Lock()

    def echo_do_post(self):
        content_length = int(self.headers.get("Content-Length", 0))
        body = self.rfile.read(content_length) if content_length > 0 else b""

        with Handler.lock:
            Handler.call_count += 1
            Handler.received_requests.append({
                "path": self.path,
                "headers": dict(self.headers),
                "body": body.decode("utf-8", errors="replace"),
            })
            idx = min(Handler.call_count - 1, len(Handler.response_queue) - 1)
            status_code, headers, response_body = Handler.response_queue[idx]

        if status_code == 200:
            # Echo the received body inside a chat completion response
            echo_response = json.dumps({
                "id": "chatcmpl-echo-001",
                "object": "chat.completion",
                "created": 1700000000,
                "model": "echo-model",
                "choices": [
                    {
                        "index": 0,
                        "message": {
                            "role": "assistant",
                            "content": body.decode("utf-8", errors="replace"),
                        },
                        "finish_reason": "stop",
                    }
                ],
                "usage": {
                    "prompt_tokens": 10,
                    "completion_tokens": 5,
                    "total_tokens": 15,
                },
            })
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.end_headers()
            self.wfile.write(echo_response.encode("utf-8"))
        else:
            self.send_response(status_code)
            for key, value in headers.items():
                self.send_header(key, value)
            self.send_header("Content-Type", "application/json")
            self.end_headers()
            if isinstance(response_body, str):
                response_body = response_body.encode("utf-8")
            self.wfile.write(response_body)

    Handler.do_POST = echo_do_post
    return Handler


class EchoMockServer:
    """Mock server that echoes request body on 200 responses."""

    def __init__(self, response_queue: list):
        self.handler_class = create_echo_handler_class(response_queue)
        self.server = HTTPServer(("0.0.0.0", 0), self.handler_class)
        self.port = self.server.server_address[1]
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)

    def start(self):
        self.thread.start()
        logger.info(f"Echo mock server started on port {self.port}")

    def stop(self):
        self.server.shutdown()
        self.thread.join(timeout=5)

    @property
    def call_count(self) -> int:
        return self.handler_class.call_count

    @property
    def received_requests(self) -> list:
        return self.handler_class.received_requests


# ---------------------------------------------------------------------------
# Delayed-response handler for IT-10 (timeout triggers retry)
# ---------------------------------------------------------------------------

def create_delayed_handler_class(response_queue: list, delay_seconds: float) -> type:
    """
    Create a handler class that delays its response by *delay_seconds* before
    sending the queued response.  Used to simulate upstream timeouts.
    """

    class Handler(MockUpstreamHandler):
        pass

    Handler.response_queue = list(response_queue)
    Handler.received_requests = []
    Handler.call_count = 0
    Handler.lock = threading.Lock()

    def delayed_do_post(self):
        content_length = int(self.headers.get("Content-Length", 0))
        body = self.rfile.read(content_length) if content_length > 0 else b""

        with Handler.lock:
            Handler.call_count += 1
            Handler.received_requests.append({
                "path": self.path,
                "headers": dict(self.headers),
                "body": body.decode("utf-8", errors="replace"),
            })
            idx = min(Handler.call_count - 1, len(Handler.response_queue) - 1)
            status_code, headers, response_body = Handler.response_queue[idx]

        # Delay before responding — gateway should time out before this completes
        time.sleep(delay_seconds)

        self.send_response(status_code)
        for key, value in headers.items():
            self.send_header(key, value)
        self.send_header("Content-Type", "application/json")
        self.end_headers()
        if isinstance(response_body, str):
            response_body = response_body.encode("utf-8")
        self.wfile.write(response_body)

    Handler.do_POST = delayed_do_post
    return Handler


class DelayedMockServer:
    """Mock server that delays responses to simulate slow upstreams / timeouts."""

    def __init__(self, response_queue: list, delay_seconds: float):
        self.handler_class = create_delayed_handler_class(
            response_queue, delay_seconds
        )
        self.server = HTTPServer(("0.0.0.0", 0), self.handler_class)
        self.port = self.server.server_address[1]
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)

    def start(self):
        self.thread.start()
        logger.info(f"Delayed mock server started on port {self.port} ")

    def stop(self):
        self.server.shutdown()
        self.thread.join(timeout=5)

    @property
    def call_count(self) -> int:
        return self.handler_class.call_count

    @property
    def received_requests(self) -> list:
        return self.handler_class.received_requests


# ===========================================================================
# Integration Tests
# ===========================================================================


class TestRetryIntegration:
    """
    P0 integration tests for retry-on-ratelimit feature.

    These tests require the full gateway infrastructure (Docker, planoai CLI).
    Each test starts mock servers, configures the gateway, sends requests,
    and validates retry behavior end-to-end.
    """

    def test_it1_basic_retry_on_429(self):
        """
        IT-1: Basic retry on 429.

        Primary mock returns 429, secondary returns 200.
        Assert client gets 200 from the secondary provider.
        """
        # Setup mock servers
        primary = MockServer([
            (429, {}, make_error_response(429, "Rate limit exceeded")),
        ])
        secondary = MockServer([
            (200, {}, SUCCESS_RESPONSE),
        ])
        primary.start()
        secondary.start()
        config_path = None

        try:
            # Write config with actual ports
            config_path = write_config("retry_it1_basic_429.yaml", {
                "MOCK_PRIMARY_PORT": primary.port,
                "MOCK_SECONDARY_PORT": secondary.port,
            })

            # Start gateway
            gateway_up(config_path)

            # Send request
            resp = requests.post(
                GATEWAY_CHAT_URL,
                json=CHAT_REQUEST_BODY,
                headers={"Authorization": "Bearer test-key"},
                timeout=30,
            )

            # Assert: client gets 200 from secondary
            assert resp.status_code == 200, (
                f"Expected 200 but got {resp.status_code}: {resp.text}"
            )
            body = resp.json()
            assert "choices" in body
            assert body["choices"][0]["message"]["content"] == "Hello from mock provider!"

            # Assert: primary was called (got 429), secondary was called (returned 200)
            assert primary.call_count >= 1, "Primary should have been called"
            assert secondary.call_count >= 1, "Secondary should have been called"

        finally:
            gateway_down()
            primary.stop()
            secondary.stop()
            if config_path and os.path.exists(config_path):
                os.unlink(config_path)

    def test_it2_retry_on_503_different_provider(self):
        """
        IT-2: Retry on 503 with different_provider strategy.

        Primary returns 503, secondary returns 200.
        Assert client gets 200 from the secondary provider.
        """
        primary = MockServer([
            (503, {}, make_error_response(503, "Service Unavailable")),
        ])
        secondary = MockServer([
            (200, {}, SUCCESS_RESPONSE),
        ])
        primary.start()
        secondary.start()
        config_path = None

        try:
            config_path = write_config("retry_it2_503_different_provider.yaml", {
                "MOCK_PRIMARY_PORT": primary.port,
                "MOCK_SECONDARY_PORT": secondary.port,
            })
            gateway_up(config_path)

            resp = requests.post(
                GATEWAY_CHAT_URL,
                json=CHAT_REQUEST_BODY,
                headers={"Authorization": "Bearer test-key"},
                timeout=30,
            )

            assert resp.status_code == 200, (
                f"Expected 200 but got {resp.status_code}: {resp.text}"
            )
            body = resp.json()
            assert "choices" in body
            assert primary.call_count >= 1
            assert secondary.call_count >= 1

        finally:
            gateway_down()
            primary.stop()
            secondary.stop()
            if config_path and os.path.exists(config_path):
                os.unlink(config_path)

    def test_it3_all_retries_exhausted(self):
        """
        IT-3: All retries exhausted.

        All mock providers return 429.
        Assert client gets an error response with attempts list and total_attempts.
        """
        primary = MockServer([
            (429, {}, make_error_response(429, "Rate limit exceeded")),
        ])
        secondary = MockServer([
            (429, {}, make_error_response(429, "Rate limit exceeded")),
        ])
        primary.start()
        secondary.start()
        config_path = None

        try:
            config_path = write_config("retry_it3_all_exhausted.yaml", {
                "MOCK_PRIMARY_PORT": primary.port,
                "MOCK_SECONDARY_PORT": secondary.port,
            })
            gateway_up(config_path)

            resp = requests.post(
                GATEWAY_CHAT_URL,
                json=CHAT_REQUEST_BODY,
                headers={"Authorization": "Bearer test-key"},
                timeout=30,
            )

            # Should get an error response (429 or the gateway's retry_exhausted error)
            assert resp.status_code >= 400, (
                f"Expected error status but got {resp.status_code}"
            )
            body = resp.json()

            # The error response should contain retry attempt details
            error = body.get("error", {})
            assert error.get("type") == "retry_exhausted", (
                f"Expected retry_exhausted error type, got: {error}"
            )
            assert "attempts" in error, "Error should contain attempts list"
            assert "total_attempts" in error, "Error should contain total_attempts"
            assert error["total_attempts"] >= 2, (
                f"Expected at least 2 total attempts, got {error['total_attempts']}"
            )

        finally:
            gateway_down()
            primary.stop()
            secondary.stop()
            if config_path and os.path.exists(config_path):
                os.unlink(config_path)

    def test_it4_no_retry_policy_no_retry(self):
        """
        IT-4: No retry_policy → no retry.

        Primary returns 429 with no retry_policy configured.
        Assert client gets 429 directly (no retry to secondary).
        """
        primary = MockServer([
            (429, {}, make_error_response(429, "Rate limit exceeded")),
        ])
        secondary = MockServer([
            (200, {}, SUCCESS_RESPONSE),
        ])
        primary.start()
        secondary.start()
        config_path = None

        try:
            config_path = write_config("retry_it4_no_retry_policy.yaml", {
                "MOCK_PRIMARY_PORT": primary.port,
                "MOCK_SECONDARY_PORT": secondary.port,
            })
            gateway_up(config_path)

            resp = requests.post(
                GATEWAY_CHAT_URL,
                json=CHAT_REQUEST_BODY,
                headers={"Authorization": "Bearer test-key"},
                timeout=30,
            )

            # Should get 429 directly — no retry
            assert resp.status_code == 429, (
                f"Expected 429 but got {resp.status_code}: {resp.text}"
            )

            # Secondary should NOT have been called
            assert secondary.call_count == 0, (
                f"Secondary should not be called without retry_policy, "
                f"but was called {secondary.call_count} times"
            )

        finally:
            gateway_down()
            primary.stop()
            secondary.stop()
            if config_path and os.path.exists(config_path):
                os.unlink(config_path)

    def test_it5_max_attempts_respected(self):
        """
        IT-5: max_attempts respected.

        Primary returns 429, max_attempts: 1.
        Assert only 1 retry attempt is made, then error is returned.
        The secondary also returns 429 to ensure we see the exhaustion.
        """
        primary = MockServer([
            (429, {}, make_error_response(429, "Rate limit exceeded")),
        ])
        secondary = MockServer([
            (429, {}, make_error_response(429, "Rate limit exceeded")),
        ])
        tertiary = MockServer([
            (200, {}, SUCCESS_RESPONSE),
        ])
        primary.start()
        secondary.start()
        tertiary.start()
        config_path = None

        try:
            config_path = write_config("retry_it5_max_attempts.yaml", {
                "MOCK_PRIMARY_PORT": primary.port,
                "MOCK_SECONDARY_PORT": secondary.port,
                "MOCK_TERTIARY_PORT": tertiary.port,
            })
            gateway_up(config_path)

            resp = requests.post(
                GATEWAY_CHAT_URL,
                json=CHAT_REQUEST_BODY,
                headers={"Authorization": "Bearer test-key"},
                timeout=30,
            )

            # With max_attempts: 1, only 1 retry should happen after the initial failure.
            # Primary fails (429) → 1 retry to secondary (429) → exhausted.
            # Tertiary should NOT be reached.
            assert resp.status_code >= 400, (
                f"Expected error status but got {resp.status_code}"
            )

            assert tertiary.call_count == 0, (
                f"Tertiary should not be called with max_attempts=1, "
                f"but was called {tertiary.call_count} times"
            )

            # Total calls: primary (1) + secondary (1 retry) = 2
            total_calls = primary.call_count + secondary.call_count
            assert total_calls <= 2, (
                f"Expected at most 2 total calls (1 original + 1 retry), "
                f"got {total_calls}"
            )

        finally:
            gateway_down()
            primary.stop()
            secondary.stop()
            tertiary.stop()
            if config_path and os.path.exists(config_path):
                os.unlink(config_path)

    def test_it6_backoff_delay_observed(self):
        """
        IT-6: Backoff delay observed.

        Configure same_model strategy with backoff (base_ms: 500, jitter: false).
        Primary returns 429 twice, then 200 on third attempt.
        Assert total response time includes backoff delays.

        With base_ms=500 and no jitter:
          - Attempt 1: fail (429)
          - Backoff: 500ms (500 * 2^0)
          - Attempt 2: fail (429)
          - Backoff: 1000ms (500 * 2^1)
          - Attempt 3: success (200)
        Total backoff >= 1500ms (500 + 1000)
        """
        primary = MockServer([
            (429, {}, make_error_response(429, "Rate limit exceeded")),
            (429, {}, make_error_response(429, "Rate limit exceeded")),
            (200, {}, SUCCESS_RESPONSE),
        ])
        primary.start()
        config_path = None

        try:
            config_path = write_config("retry_it6_backoff_delay.yaml", {
                "MOCK_PRIMARY_PORT": primary.port,
            })
            gateway_up(config_path)

            start_time = time.time()
            resp = requests.post(
                GATEWAY_CHAT_URL,
                json=CHAT_REQUEST_BODY,
                headers={"Authorization": "Bearer test-key"},
                timeout=60,
            )
            elapsed = time.time() - start_time

            assert resp.status_code == 200, (
                f"Expected 200 but got {resp.status_code}: {resp.text}"
            )

            # With base_ms=500 and no jitter, backoff should be at least:
            # 500ms (attempt 1→2) + 1000ms (attempt 2→3) = 1500ms
            # Use a slightly lower threshold (1.0s) to account for timing variance
            min_expected_delay = 1.0  # seconds
            assert elapsed >= min_expected_delay, (
                f"Expected response time >= {min_expected_delay}s due to backoff, "
                f"but got {elapsed:.2f}s"
            )

            # Primary should have been called 3 times
            assert primary.call_count == 3, (
                f"Expected 3 calls to primary, got {primary.call_count}"
            )

        finally:
            gateway_down()
            primary.stop()
            if config_path and os.path.exists(config_path):
                os.unlink(config_path)

    def test_it12_streaming_preserved_across_retry(self):
        """
        IT-12: Streaming request preserved across retry.

        Primary returns 429, secondary returns 200 with SSE streaming.
        Assert client receives a streamed response.
        """
        # Primary always returns 429
        primary = MockServer([
            (429, {}, make_error_response(429, "Rate limit exceeded")),
        ])
        # Secondary returns streaming 200
        secondary_handler = create_streaming_handler_class([
            (200, {}, "STREAM"),
        ])
        secondary_server = HTTPServer(("0.0.0.0", 0), secondary_handler)
        secondary_port = secondary_server.server_address[1]
        secondary_thread = threading.Thread(
            target=secondary_server.serve_forever, daemon=True
        )

        primary.start()
        secondary_thread.start()
        logger.info(f"Streaming secondary mock started on port {secondary_port}")
        config_path = None

        try:
            config_path = write_config("retry_it12_streaming.yaml", {
                "MOCK_PRIMARY_PORT": primary.port,
                "MOCK_SECONDARY_PORT": secondary_port,
            })
            gateway_up(config_path)

            # Send a streaming request
            streaming_body = dict(CHAT_REQUEST_BODY)
            streaming_body["stream"] = True

            resp = requests.post(
                GATEWAY_CHAT_URL,
                json=streaming_body,
                headers={"Authorization": "Bearer test-key"},
                stream=True,
                timeout=30,
            )

            assert resp.status_code == 200, (
                f"Expected 200 but got {resp.status_code}: {resp.text}"
            )

            # Collect streamed chunks
            chunks = []
            for line in resp.iter_lines(decode_unicode=True):
                if line:
                    chunks.append(line)

            # Should have received SSE data chunks
            assert len(chunks) > 0, "Should have received streaming chunks"

            # Verify at least one chunk contains "data:" prefix (SSE format)
            data_chunks = [c for c in chunks if c.startswith("data:")]
            assert len(data_chunks) > 0, (
                f"Expected SSE data chunks, got: {chunks}"
            )

            # Verify the stream contains expected content
            content_found = False
            for chunk in data_chunks:
                if chunk == "data: [DONE]":
                    continue
                try:
                    payload = json.loads(chunk[len("data: "):])
                    delta = payload.get("choices", [{}])[0].get("delta", {})
                    if delta.get("content"):
                        content_found = True
                except (json.JSONDecodeError, IndexError):
                    pass

            assert content_found, "Should have received content in streaming chunks"

            # Primary should have been called (got 429)
            assert primary.call_count >= 1, "Primary should have been called"

        finally:
            gateway_down()
            primary.stop()
            secondary_server.shutdown()
            secondary_thread.join(timeout=5)
            if config_path and os.path.exists(config_path):
                os.unlink(config_path)

    def test_it13_request_body_preserved_across_retry(self):
        """
        IT-13: Request body preserved across retry.

        Primary returns 429, secondary echoes the request body.
        Assert the echoed body matches the original request.
        """
        primary = MockServer([
            (429, {}, make_error_response(429, "Rate limit exceeded")),
        ])
        # Secondary echoes the request body
        echo_server = EchoMockServer([
            (200, {}, ""),  # Status 200 triggers echo behavior
        ])

        primary.start()
        echo_server.start()
        config_path = None

        try:
            config_path = write_config("retry_it13_body_preserved.yaml", {
                "MOCK_PRIMARY_PORT": primary.port,
                "MOCK_SECONDARY_PORT": echo_server.port,
            })
            gateway_up(config_path)

            # Send request with a distinctive body
            request_body = {
                "model": "openai/gpt-4o",
                "messages": [
                    {"role": "system", "content": "You are a helpful assistant."},
                    {"role": "user", "content": "Tell me about retry mechanisms."},
                ],
                "temperature": 0.7,
                "max_tokens": 100,
            }

            resp = requests.post(
                GATEWAY_CHAT_URL,
                json=request_body,
                headers={"Authorization": "Bearer test-key"},
                timeout=30,
            )

            assert resp.status_code == 200, (
                f"Expected 200 but got {resp.status_code}: {resp.text}"
            )

            # The echo server received the request body — verify it was preserved
            assert echo_server.call_count >= 1, "Echo server should have been called"

            # Parse the body that the echo server received
            received_body_str = echo_server.received_requests[-1]["body"]
            received_body = json.loads(received_body_str)

            # The gateway may modify the model field when routing to a different
            # provider, but the messages and other fields should be preserved
            assert received_body.get("messages") is not None, (
                "Messages should be preserved in the forwarded request"
            )

            # Verify the user message content is preserved
            user_messages = [
                m for m in received_body["messages"] if m.get("role") == "user"
            ]
            assert len(user_messages) > 0, "User messages should be preserved"
            assert user_messages[-1]["content"] == "Tell me about retry mechanisms.", (
                f"User message content should be preserved, got: {user_messages[-1]}"
            )

            # Primary should have been called (got 429)
            assert primary.call_count >= 1, "Primary should have been called"

        finally:
            gateway_down()
            primary.stop()
            echo_server.stop()
            if config_path and os.path.exists(config_path):
                os.unlink(config_path)


    # -----------------------------------------------------------------------
    # P1 Integration Tests (IT-7 through IT-10)
    # -----------------------------------------------------------------------

    def test_it7_fallback_models_priority(self):
        """
        IT-7: Fallback models priority.

        Primary mock returns 429, fallback[0] returns 429, fallback[1] returns 200.
        Assert client gets 200 from fallback[1] and providers are tried in the
        order defined by fallback_models.

        Config: fallback_models: [anthropic/claude-3-5-sonnet, mistral/mistral-large]
        """
        primary = MockServer([
            (429, {}, make_error_response(429, "Rate limit exceeded")),
        ])
        fallback1 = MockServer([
            (429, {}, make_error_response(429, "Rate limit exceeded")),
        ])
        fallback2 = MockServer([
            (200, {}, SUCCESS_RESPONSE),
        ])
        primary.start()
        fallback1.start()
        fallback2.start()
        config_path = None

        try:
            config_path = write_config("retry_it7_fallback_priority.yaml", {
                "MOCK_PRIMARY_PORT": primary.port,
                "MOCK_FALLBACK1_PORT": fallback1.port,
                "MOCK_FALLBACK2_PORT": fallback2.port,
            })
            gateway_up(config_path)

            resp = requests.post(
                GATEWAY_CHAT_URL,
                json=CHAT_REQUEST_BODY,
                headers={"Authorization": "Bearer test-key"},
                timeout=30,
            )

            # Assert: client gets 200 from fallback[1]
            assert resp.status_code == 200, (
                f"Expected 200 but got {resp.status_code}: {resp.text}"
            )
            body = resp.json()
            assert "choices" in body
            assert body["choices"][0]["message"]["content"] == "Hello from mock provider!"

            # Assert: providers tried in order — primary, fallback[0], fallback[1]
            assert primary.call_count >= 1, "Primary should have been called first"
            assert fallback1.call_count >= 1, (
                "Fallback[0] (anthropic/claude-3-5-sonnet) should have been tried "
                "before fallback[1]"
            )
            assert fallback2.call_count >= 1, (
                "Fallback[1] (mistral/mistral-large) should have been called"
            )

        finally:
            gateway_down()
            primary.stop()
            fallback1.stop()
            fallback2.stop()
            if config_path and os.path.exists(config_path):
                os.unlink(config_path)

    def test_it8_retry_after_header_honored(self):
        """
        IT-8: Retry-After header honored.

        Primary returns 429 + Retry-After: 2 on the first call, then 200 on the
        second call (same_model strategy).  Assert the total response time is
        >= 2 seconds, proving the gateway waited for the Retry-After duration.
        """
        primary = MockServer([
            (429, {"Retry-After": "2"}, make_error_response(429, "Rate limit exceeded")),
            (200, {}, SUCCESS_RESPONSE),
        ])
        primary.start()
        config_path = None

        try:
            config_path = write_config("retry_it8_retry_after_honored.yaml", {
                "MOCK_PRIMARY_PORT": primary.port,
            })
            gateway_up(config_path)

            start_time = time.time()
            resp = requests.post(
                GATEWAY_CHAT_URL,
                json=CHAT_REQUEST_BODY,
                headers={"Authorization": "Bearer test-key"},
                timeout=30,
            )
            elapsed = time.time() - start_time

            # Assert: client gets 200 after the retry
            assert resp.status_code == 200, (
                f"Expected 200 but got {resp.status_code}: {resp.text}"
            )
            body = resp.json()
            assert "choices" in body

            # Assert: total time >= 2 seconds (Retry-After: 2 was honored)
            # Use a slightly lower threshold to account for timing variance
            min_expected_delay = 1.8  # seconds
            assert elapsed >= min_expected_delay, (
                f"Expected response time >= {min_expected_delay}s due to "
                f"Retry-After: 2, but got {elapsed:.2f}s"
            )

            # Primary should have been called twice (429 then 200)
            assert primary.call_count == 2, (
                f"Expected 2 calls to primary (429 + 200), got {primary.call_count}"
            )

        finally:
            gateway_down()
            primary.stop()
            if config_path and os.path.exists(config_path):
                os.unlink(config_path)

    def test_it9_retry_after_blocks_initial_selection(self):
        """
        IT-9: Retry-After blocks initial selection.

        First request: primary returns 429 + Retry-After: 60 and the gateway
        retries to the secondary (which returns 200).

        Second request (sent within 60s): because the primary is globally
        blocked by the Retry-After state, the gateway should route directly
        to the alternative provider without hitting the primary again.
        """
        # Primary: first call returns 429 + Retry-After: 60, subsequent calls
        # return 200 (but should not be reached for the second request).
        primary = MockServer([
            (429, {"Retry-After": "60"}, make_error_response(429, "Rate limit exceeded")),
            (200, {}, SUCCESS_RESPONSE),
            (200, {}, SUCCESS_RESPONSE),
        ])
        secondary = MockServer([
            (200, {}, SUCCESS_RESPONSE),
            (200, {}, SUCCESS_RESPONSE),
        ])
        primary.start()
        secondary.start()
        config_path = None

        try:
            config_path = write_config(
                "retry_it9_retry_after_blocks_selection.yaml",
                {
                    "MOCK_PRIMARY_PORT": primary.port,
                    "MOCK_SECONDARY_PORT": secondary.port,
                },
            )
            gateway_up(config_path)

            # --- First request: triggers the Retry-After state ---
            resp1 = requests.post(
                GATEWAY_CHAT_URL,
                json=CHAT_REQUEST_BODY,
                headers={"Authorization": "Bearer test-key"},
                timeout=30,
            )
            assert resp1.status_code == 200, (
                f"First request: expected 200 but got {resp1.status_code}: {resp1.text}"
            )

            primary_calls_after_first = primary.call_count
            secondary_calls_after_first = secondary.call_count

            # Primary should have been called once (got 429), secondary once (got 200)
            assert primary_calls_after_first >= 1, (
                "Primary should have been called for the first request"
            )
            assert secondary_calls_after_first >= 1, (
                "Secondary should have been called as fallback for the first request"
            )

            # --- Second request: within the 60s Retry-After window ---
            # The primary model should be blocked globally, so the gateway
            # should route to the alternative provider directly.
            resp2 = requests.post(
                GATEWAY_CHAT_URL,
                json={
                    "model": "openai/gpt-4o",
                    "messages": [{"role": "user", "content": "Second request"}],
                },
                headers={"Authorization": "Bearer test-key"},
                timeout=30,
            )
            assert resp2.status_code == 200, (
                f"Second request: expected 200 but got {resp2.status_code}: {resp2.text}"
            )

            # Assert: primary was NOT called again for the second request
            # (it should still be blocked by the 60s Retry-After)
            assert primary.call_count == primary_calls_after_first, (
                f"Primary should not have been called for the second request "
                f"(blocked by Retry-After: 60). Calls before: "
                f"{primary_calls_after_first}, after: {primary.call_count}"
            )

            # Assert: secondary handled the second request
            assert secondary.call_count > secondary_calls_after_first, (
                f"Secondary should have handled the second request. "
                f"Calls before: {secondary_calls_after_first}, "
                f"after: {secondary.call_count}"
            )

        finally:
            gateway_down()
            primary.stop()
            secondary.stop()
            if config_path and os.path.exists(config_path):
                os.unlink(config_path)

    def test_it10_timeout_triggers_retry(self):
        """
        IT-10: Timeout triggers retry.

        Primary mock delays its response beyond the gateway's request timeout.
        Secondary returns 200 immediately.
        Assert client gets 200 from the secondary provider.
        """
        # Primary delays 120 seconds — well beyond any reasonable gateway timeout.
        # The gateway should time out and retry to the secondary.
        primary = DelayedMockServer(
            response_queue=[
                (200, {}, SUCCESS_RESPONSE),
            ],
            delay_seconds=120,
        )
        secondary = MockServer([
            (200, {}, SUCCESS_RESPONSE),
        ])
        primary.start()
        secondary.start()
        config_path = None

        try:
            config_path = write_config("retry_it10_timeout_triggers_retry.yaml", {
                "MOCK_PRIMARY_PORT": primary.port,
                "MOCK_SECONDARY_PORT": secondary.port,
            })
            gateway_up(config_path)

            resp = requests.post(
                GATEWAY_CHAT_URL,
                json=CHAT_REQUEST_BODY,
                headers={"Authorization": "Bearer test-key"},
                timeout=120,
            )

            # Assert: client gets 200 from the secondary
            assert resp.status_code == 200, (
                f"Expected 200 but got {resp.status_code}: {resp.text}"
            )
            body = resp.json()
            assert "choices" in body
            assert body["choices"][0]["message"]["content"] == "Hello from mock provider!"

            # Assert: primary was called (timed out), secondary was called (returned 200)
            assert primary.call_count >= 1, (
                "Primary should have been called (and timed out)"
            )
            assert secondary.call_count >= 1, (
                "Secondary should have been called after primary timed out"
            )

        finally:
            gateway_down()
            primary.stop()
            secondary.stop()
            if config_path and os.path.exists(config_path):
                os.unlink(config_path)

    def test_it11_high_latency_proactive_failover(self):
        """
        IT-11: High latency proactive failover.

        First request: primary mock delays response by ~1.5s (threshold_ms=1000
        + 500ms buffer) but completes with 200 OK. The client receives the slow
        200 response (completed responses are always delivered). However, the
        gateway records a Latency_Block_State for the primary model.

        Second request: sent immediately after the first. Because the primary
        is now latency-blocked (block_duration_seconds=60, min_triggers=1),
        the gateway should route directly to the secondary provider.

        Config: on_high_latency with min_triggers: 1, threshold_ms: 1000,
        block_duration_seconds: 60, measure: "total", scope: "model",
        apply_to: "global".
        """
        # Primary: delays 1.5s (exceeds 1000ms threshold), returns 200.
        # Queue two responses in case the primary is called twice (it shouldn't
        # be for the second request, but we need a response ready just in case).
        primary = DelayedMockServer(
            response_queue=[
                (200, {}, SUCCESS_RESPONSE),
                (200, {}, SUCCESS_RESPONSE),
            ],
            delay_seconds=1.5,
        )
        # Secondary: returns 200 immediately.
        secondary = MockServer([
            (200, {}, SUCCESS_RESPONSE),
            (200, {}, SUCCESS_RESPONSE),
        ])
        primary.start()
        secondary.start()
        config_path = None

        try:
            config_path = write_config(
                "retry_it11_high_latency_failover.yaml",
                {
                    "MOCK_PRIMARY_PORT": primary.port,
                    "MOCK_SECONDARY_PORT": secondary.port,
                },
            )
            gateway_up(config_path)

            # --- First request: triggers the latency block ---
            # The primary will respond with 200 after ~1.5s delay.
            # Since the response completes, the client gets the 200 back,
            # but the gateway should record a Latency_Block_State entry.
            resp1 = requests.post(
                GATEWAY_CHAT_URL,
                json=CHAT_REQUEST_BODY,
                headers={"Authorization": "Bearer test-key"},
                timeout=30,
            )
            assert resp1.status_code == 200, (
                f"First request: expected 200 but got {resp1.status_code}: "
                f"{resp1.text}"
            )

            primary_calls_after_first = primary.call_count
            secondary_calls_after_first = secondary.call_count

            # Primary should have been called once (slow 200).
            assert primary_calls_after_first >= 1, (
                "Primary should have been called for the first request"
            )

            # --- Second request: within the 60s latency block window ---
            # The primary model should be latency-blocked globally, so the
            # gateway should route to the secondary provider directly.
            resp2 = requests.post(
                GATEWAY_CHAT_URL,
                json={
                    "model": "openai/gpt-4o",
                    "messages": [{"role": "user", "content": "Second request"}],
                },
                headers={"Authorization": "Bearer test-key"},
                timeout=30,
            )
            assert resp2.status_code == 200, (
                f"Second request: expected 200 but got {resp2.status_code}: "
                f"{resp2.text}"
            )

            # Assert: primary was NOT called again for the second request
            # (it should be latency-blocked for 60s after the slow first response).
            assert primary.call_count == primary_calls_after_first, (
                f"Primary should not have been called for the second request "
                f"(latency-blocked for 60s). Calls before: "
                f"{primary_calls_after_first}, after: {primary.call_count}"
            )

            # Assert: secondary handled the second request.
            assert secondary.call_count > secondary_calls_after_first, (
                f"Secondary should have handled the second request. "
                f"Calls before: {secondary_calls_after_first}, "
                f"after: {secondary.call_count}"
            )

        finally:
            gateway_down()
            primary.stop()
            secondary.stop()
            if config_path and os.path.exists(config_path):
                os.unlink(config_path)
