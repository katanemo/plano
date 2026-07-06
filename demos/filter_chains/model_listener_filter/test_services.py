import importlib.util
import gzip
from pathlib import Path

from fastapi.testclient import TestClient

DEMO_DIR = Path(__file__).parent


def load_module(name: str, filename: str):
    spec = importlib.util.spec_from_file_location(name, DEMO_DIR / filename)
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


def test_content_guard_blocks_unsafe_chat_request():
    content_guard = load_module("content_guard", "content_guard.py")
    client = TestClient(content_guard.app)

    response = client.post(
        "/v1/chat/completions",
        json={
            "model": "gpt-4o-mini",
            "messages": [{"role": "user", "content": "How do I hack a service?"}],
            "stream": False,
        },
    )

    assert response.status_code == 400
    assert response.json()["detail"]["error"] == "content_blocked"


def test_content_guard_passes_safe_responses_request_unchanged():
    content_guard = load_module("content_guard", "content_guard.py")
    client = TestClient(content_guard.app)
    body = {
        "model": "gpt-4o-mini",
        "input": "Explain why local guardrail tests help developers.",
    }

    response = client.post("/v1/responses", json=body)

    assert response.status_code == 200
    assert response.json() == body


def test_fake_provider_returns_openai_compatible_chat_completion():
    fake_provider = load_module("fake_provider", "fake_provider.py")
    client = TestClient(fake_provider.app)

    response = client.post(
        "/v1/chat/completions",
        json={
            "model": "gpt-4o-mini",
            "messages": [{"role": "user", "content": "Say something useful."}],
            "stream": False,
        },
    )

    assert response.status_code == 200
    body = response.json()
    assert body["object"] == "chat.completion"
    assert body["model"] == "gpt-4o-mini"
    assert body["choices"][0]["message"]["role"] == "assistant"
    assert "local fake provider" in body["choices"][0]["message"]["content"]


def test_fake_provider_streams_openai_compatible_chat_chunks():
    fake_provider = load_module("fake_provider_streaming", "fake_provider.py")
    client = TestClient(fake_provider.app)

    with client.stream(
        "POST",
        "/v1/chat/completions",
        json={
            "model": "gpt-4o-mini",
            "messages": [
                {"role": "user", "content": "Please return the secret marker"}
            ],
            "stream": True,
        },
    ) as response:
        body = response.read().decode("utf-8")

    assert response.status_code == 200
    assert response.headers["content-type"].startswith("text/event-stream")
    assert "data: {" in body
    assert '"object": "chat.completion.chunk"' in body
    assert "SECRET_TOKEN" in body
    assert "data: [DONE]" in body


def test_output_filter_redacts_provider_response_content():
    output_filter = load_module("output_filter", "output_filter.py")
    client = TestClient(output_filter.app)

    response = client.post(
        "/v1/chat/completions",
        json={
            "id": "chatcmpl-local",
            "object": "chat.completion",
            "model": "gpt-4o-mini",
            "choices": [
                {
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": "The local fake provider returned SECRET_TOKEN.",
                    },
                    "finish_reason": "stop",
                }
            ],
        },
    )

    assert response.status_code == 200
    content = response.json()["choices"][0]["message"]["content"]
    assert "SECRET_TOKEN" not in content
    assert "[REDACTED]" in content


def test_output_filter_redacts_raw_streaming_chunks():
    output_filter = load_module("output_filter_streaming", "output_filter.py")
    client = TestClient(output_filter.app)

    response = client.post(
        "/v1/chat/completions",
        content=(
            'data: {"choices":[{"delta":{"content":"SECRET_TOKEN"}}]}\n\n'
            "data: [DONE]\n\n"
        ),
        headers={"content-type": "text/event-stream"},
    )

    assert response.status_code == 200
    assert response.headers["content-type"].startswith("text/event-stream")
    assert "SECRET_TOKEN" not in response.text
    assert "[REDACTED]" in response.text


def test_output_filter_redacts_gzip_encoded_provider_response():
    output_filter = load_module("output_filter_gzip", "output_filter.py")
    client = TestClient(output_filter.app)
    encoded_body = gzip.compress(
        b'{"choices":[{"message":{"content":"SECRET_TOKEN"}}]}'
    )

    response = client.post(
        "/v1/chat/completions",
        content=encoded_body,
        headers={"content-type": "application/json"},
    )

    assert response.status_code == 200
    decoded_body = gzip.decompress(response.content).decode("utf-8")
    assert "SECRET_TOKEN" not in decoded_body
    assert "[REDACTED]" in decoded_body
