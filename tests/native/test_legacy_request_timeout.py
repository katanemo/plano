"""Opt-in real Envoy/WASM regression, with only HTTP upstream dependencies faked.

Build both WASM plugins, then run from cli/:
PLANO_TEST_ENVOY=/path/to/envoy uv run pytest ../tests/native/test_legacy_request_timeout.py
No model, provider key, Docker daemon, or binary download is used by this test.
"""

import gzip
import json
import os
import socket
import subprocess
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

import pytest
import requests
import yaml

from planoai import native_runner
from planoai.native_binaries import _find_local_wasm_plugins


class Upstream(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def do_POST(self):
        if "chunked" in self.headers.get("transfer-encoding", "").lower():
            parts = []
            while True:
                line = self.rfile.readline()
                if not line:
                    return
                size = int(line.split(b";", 1)[0].strip(), 16)
                if size == 0:
                    while self.rfile.readline().strip():
                        pass
                    break
                parts.append(self.rfile.read(size))
                assert self.rfile.read(2) == b"\r\n"
            raw = b"".join(parts)
        else:
            raw = self.rfile.read(int(self.headers.get("content-length", 0)))
        if self.headers.get("content-encoding") == "gzip":
            raw = gzip.decompress(raw)
        content = json.dumps(json.loads(raw))
        selection = self.path == "/function_calling"
        delayed = (
            "selection-slow" in content
            if selection
            else "slow" in content or self.path == "/tool"
        )
        if delayed:
            time.sleep(self.server.delay)
        message = {"role": "assistant", "content": "" if selection else "ok"}
        if selection and "tool-slow" in content:
            message["tool_calls"] = [
                {
                    "id": "test-call",
                    "type": "function",
                    "function": {"name": "test_tool", "arguments": "{}"},
                }
            ]
        data = json.dumps(
            {
                "model": "Arch-Function" if selection else "api_server-test",
                "choices": [{"message": message, "index": 0, "finish_reason": "stop"}],
            }
        ).encode()
        try:
            self.send_response(200)
            self.send_header("content-type", "application/json")
            self.send_header("content-length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)
        except (BrokenPipeError, ConnectionResetError):
            pass  # Expected when an actual request budget expires.


@pytest.mark.parametrize("budget,delay", [("40s", 31.5), ("1s", 2)])
def test_real_legacy_callout_budget(tmp_path, monkeypatch, budget, delay):
    envoy = os.environ.get("PLANO_TEST_ENVOY")
    if not envoy or not _find_local_wasm_plugins():
        pytest.skip("Set PLANO_TEST_ENVOY and build both local release WASM plugins")
    for port in (9091, 18621, 9901, 10000, 10001, 11000, 12000, 12001):
        with socket.socket() as probe:
            assert (
                probe.connect_ex(("127.0.0.1", port)) != 0
            ), f"port {port} is occupied"
    servers = [
        ThreadingHTTPServer(("127.0.0.1", port), Upstream) for port in (9091, 18621)
    ]
    for server in servers:
        server.daemon_threads = True
        server.delay = delay
        threading.Thread(target=server.serve_forever, daemon=True).start()
    proc = None
    try:
        config = {
            "version": "v0.4.0",
            "request_timeout": budget,
            "listeners": [
                {
                    "type": "prompt",
                    "name": "prompt",
                    "port": 10000,
                    "address": "127.0.0.1",
                }
            ],
            "model_providers": [
                {
                    "model": "openai/test",
                    "base_url": "http://127.0.0.1:9091",
                    "access_key": "test-key",
                    "default": True,
                }
            ],
            "endpoints": {
                "test_target": {"endpoint": "127.0.0.1:18621", "protocol": "http"}
            },
            "prompt_targets": [
                {
                    "name": "test_default",
                    "description": "Test default target",
                    "default": True,
                    "endpoint": {
                        "name": "test_target",
                        "path": "/default",
                        "http_method": "POST",
                    },
                    "auto_llm_dispatch_on_response": False,
                },
                {
                    "name": "test_tool",
                    "description": "Test API tool",
                    "endpoint": {
                        "name": "test_target",
                        "path": "/tool",
                        "http_method": "POST",
                    },
                    "auto_llm_dispatch_on_response": False,
                },
            ],
        }
        if budget == "1s":
            # Keep the outer cap larger: expiry must come from the callout budget.
            config["listeners"][0]["timeout"] = "40s"
            config["prompt_targets"][1]["endpoint"]["http_headers"] = {
                "x-envoy-upstream-rq-timeout-ms": "90000",
                "X-Envoy-Upstream-Rq-Timeout-Ms": "90000",
            }
        source = tmp_path / "plano.yaml"
        source.write_text(yaml.safe_dump(config))
        monkeypatch.setattr(native_runner, "PLANO_RUN_DIR", str(tmp_path / "run"))
        envoy_config, _ = native_runner.render_native_config(str(source), {})
        with (tmp_path / "envoy.log").open("w") as log:
            proc = subprocess.Popen(
                [
                    envoy,
                    "-c",
                    envoy_config,
                    "--concurrency",
                    "2",
                    "--component-log-level",
                    "wasm:debug",
                ],
                stdout=log,
                stderr=log,
            )
            with requests.Session() as client:
                client.trust_env = False  # Loopback must not use an ambient HTTP proxy.
                for _ in range(100):
                    assert proc.poll() is None, (tmp_path / "envoy.log").read_text()
                    try:
                        if (
                            client.get(
                                "http://127.0.0.1:10000/healthz", timeout=0.5
                            ).status_code
                            == 200
                        ):
                            break
                    except requests.RequestException:
                        pass
                    time.sleep(0.1)
                else:
                    pytest.fail("native Envoy did not become healthy")
                cases = ["fast", "default-slow"]
                if budget == "1s":
                    cases += ["selection-slow", "tool-slow"]
                for content in cases:
                    start = time.monotonic()
                    response = client.post(
                        "http://127.0.0.1:10000/v1/chat/completions",
                        json={
                            "model": "openai/test",
                            "stream": False,
                            "messages": [{"role": "user", "content": content}],
                        },
                        timeout=45 if budget == "40s" else 4,
                    )
                    elapsed = time.monotonic() - start
                    if content == "fast" or budget == "40s":
                        assert response.status_code == 200, response.text
                        if content != "fast":
                            assert elapsed >= delay
                    else:
                        assert response.status_code == 504, response.text
                        assert 0.8 <= elapsed < 2, elapsed
                dispatches = [
                    line
                    for line in (tmp_path / "envoy.log").read_text().splitlines()
                    if "dispatching http call with args=CallArgs" in line
                ]
                paths = ["/function_calling", "/default"]
                if budget == "1s":
                    paths.append("/tool")
                millis = str(int(budget[:-1]) * 1000)
                for path in paths:
                    assert any(
                        f'path: "{path}"' in line
                        and f'("x-envoy-upstream-rq-timeout-ms", "{millis}")' in line
                        and f"timeout: {budget}" in line
                        for line in dispatches
                    ), dispatches
    finally:
        if proc is not None and proc.poll() is None:
            proc.terminate()
            try:
                proc.wait(timeout=10)
            except subprocess.TimeoutExpired:
                proc.kill()
                proc.wait(timeout=5)
        for server in servers:
            server.shutdown()
            server.server_close()
