"""Tests for the Peyeeye Plano filter.

The Peyeeye API (`https://api.peyeeye.ai`) is fully mocked with `respx`. Every
new branch in `peyeeye.py` should have at least one test below.
"""

from __future__ import annotations

import json
from typing import Any, Dict

import httpx
import pytest
import respx
from fastapi.testclient import TestClient

from peyeeye import (
    PEyeEyeClient,
    PEyeEyeMissingSecrets,
    create_app,
    iter_request_texts,
    iter_response_texts,
    set_request_text,
)

# ------------------------------------------------------------------- fixtures


@pytest.fixture
def api_base() -> str:
    return "https://api.peyeeye.ai"


@pytest.fixture
def client(monkeypatch, api_base) -> PEyeEyeClient:
    monkeypatch.setenv("PEYEEYE_API_KEY", "pk_test_123")
    monkeypatch.delenv("PEYEEYE_API_BASE", raising=False)
    monkeypatch.delenv("PEYEEYE_LOCALE", raising=False)
    monkeypatch.delenv("PEYEEYE_ENTITIES", raising=False)
    monkeypatch.delenv("PEYEEYE_SESSION_MODE", raising=False)
    return PEyeEyeClient()


@pytest.fixture
def app_client(client) -> TestClient:
    return TestClient(create_app(client=client))


# --------------------------------------------------------------- client tests


def test_client_missing_api_key(monkeypatch):
    monkeypatch.delenv("PEYEEYE_API_KEY", raising=False)
    with pytest.raises(PEyeEyeMissingSecrets):
        PEyeEyeClient()


def test_client_invalid_session_mode(monkeypatch):
    monkeypatch.setenv("PEYEEYE_API_KEY", "pk_test_123")
    with pytest.raises(ValueError):
        PEyeEyeClient(session_mode="bogus")


def test_client_picks_up_env_entities(monkeypatch):
    monkeypatch.setenv("PEYEEYE_API_KEY", "pk_test_123")
    monkeypatch.setenv("PEYEEYE_ENTITIES", "EMAIL, SSN ,CREDIT_CARD")
    c = PEyeEyeClient()
    assert c.entities == ["EMAIL", "SSN", "CREDIT_CARD"]


# -------------------------------------------------------------- iter helpers


def test_iter_request_texts_chat_string():
    body = {"messages": [{"role": "user", "content": "hello jane@example.com"}]}
    parts = iter_request_texts(body, "/v1/chat/completions")
    assert parts == [(("messages", 0, "content"), "hello jane@example.com")]


def test_iter_request_texts_chat_multimodal():
    body = {
        "messages": [
            {
                "role": "user",
                "content": [
                    {"type": "text", "text": "hi"},
                    {"type": "image_url", "image_url": {"url": "..."}},
                    {"type": "text", "text": "ssn 111-22-3333"},
                ],
            }
        ]
    }
    parts = iter_request_texts(body, "/v1/chat/completions")
    assert parts == [
        (("messages", 0, "content", 0, "text"), "hi"),
        (("messages", 0, "content", 2, "text"), "ssn 111-22-3333"),
    ]


def test_iter_request_texts_skips_non_user_roles():
    body = {
        "messages": [
            {"role": "system", "content": "you are helpful"},
            {"role": "user", "content": "hi"},
            {"role": "assistant", "content": "hello"},
        ]
    }
    parts = iter_request_texts(body, "/v1/chat/completions")
    assert len(parts) == 1
    assert parts[0][1] == "hi"


def test_iter_request_texts_responses_string():
    body = {"input": "hello jane@example.com"}
    parts = iter_request_texts(body, "/v1/responses")
    assert parts == [(("input",), "hello jane@example.com")]


def test_iter_request_texts_responses_list():
    body = {
        "input": [
            {"role": "system", "content": "ignored"},
            {"role": "user", "content": "hi"},
            {
                "role": "user",
                "content": [{"type": "text", "text": "ssn 111-22-3333"}],
            },
        ]
    }
    parts = iter_request_texts(body, "/v1/responses")
    assert parts == [
        (("input", 1, "content"), "hi"),
        (("input", 2, "content", 0, "text"), "ssn 111-22-3333"),
    ]


def test_set_request_text_round_trip():
    body: Dict[str, Any] = {"messages": [{"role": "user", "content": "raw"}]}
    set_request_text(body, ("messages", 0, "content"), "redacted")
    assert body["messages"][0]["content"] == "redacted"


def test_iter_response_texts_chat():
    body = {"choices": [{"message": {"content": "Email [EMAIL_0] back."}}]}
    parts = iter_response_texts(body, "/v1/chat/completions")
    assert parts == [(("choices", 0, "message", "content"), "Email [EMAIL_0] back.")]


def test_iter_response_texts_anthropic():
    body = {"content": [{"type": "text", "text": "Reach [EMAIL_0]"}]}
    parts = iter_response_texts(body, "/v1/messages")
    assert parts == [(("content", 0, "text"), "Reach [EMAIL_0]")]


# ------------------------------------------------------------ /redact endpoint


@respx.mock
def test_redact_chat_happy_path(app_client, api_base):
    respx.post(f"{api_base}/v1/redact").mock(
        return_value=httpx.Response(
            200,
            json={
                "text": ["hello [EMAIL_0]"],
                "session_id": "ses_abc",
            },
        )
    )

    resp = app_client.post(
        "/redact/v1/chat/completions",
        json={
            "model": "gpt-4o-mini",
            "messages": [{"role": "user", "content": "hello jane@example.com"}],
        },
        headers={"x-request-id": "req-1"},
    )
    assert resp.status_code == 200
    body = resp.json()
    assert body["messages"][0]["content"] == "hello [EMAIL_0]"


@respx.mock
def test_redact_no_pii_no_session_cached(app_client, api_base):
    # Even with no text-bearing chunks, the body is returned untouched.
    resp = app_client.post(
        "/redact/v1/chat/completions",
        json={"model": "gpt-4o-mini", "messages": []},
        headers={"x-request-id": "req-empty"},
    )
    assert resp.status_code == 200
    # No call should have been made to peyeeye.
    assert not respx.routes


@respx.mock
def test_redact_length_guard_fails_closed(app_client, api_base):
    """If the API returns a different number of texts, refuse to forward."""
    respx.post(f"{api_base}/v1/redact").mock(
        return_value=httpx.Response(
            200,
            json={"text": ["only one"], "session_id": "ses_x"},
        )
    )
    resp = app_client.post(
        "/redact/v1/chat/completions",
        json={
            "model": "gpt-4o-mini",
            "messages": [
                {"role": "user", "content": "first jane@example.com"},
                {"role": "user", "content": "second 555-123-4567"},
            ],
        },
        headers={"x-request-id": "req-bad-len"},
    )
    assert resp.status_code == 502
    assert "refusing to forward" in resp.json()["detail"]


@respx.mock
def test_redact_unexpected_shape_fails_closed(app_client, api_base):
    respx.post(f"{api_base}/v1/redact").mock(
        return_value=httpx.Response(200, json={"text": 42, "session_id": "ses_x"})
    )
    resp = app_client.post(
        "/redact/v1/chat/completions",
        json={
            "model": "gpt-4o-mini",
            "messages": [{"role": "user", "content": "x"}],
        },
        headers={"x-request-id": "req-bad-shape"},
    )
    assert resp.status_code == 502
    assert "unexpected response shape" in resp.json()["detail"]


@respx.mock
def test_redact_5xx_fails_closed(app_client, api_base):
    respx.post(f"{api_base}/v1/redact").mock(
        return_value=httpx.Response(500, text="boom")
    )
    resp = app_client.post(
        "/redact/v1/chat/completions",
        json={
            "model": "gpt-4o-mini",
            "messages": [{"role": "user", "content": "x"}],
        },
        headers={"x-request-id": "req-5xx"},
    )
    assert resp.status_code == 502
    assert "HTTP 500" in resp.json()["detail"]


@respx.mock
def test_redact_401_fails_closed_with_500(app_client, api_base):
    """Auth errors surface as PEyeEyeMissingSecrets -> HTTP 500."""
    respx.post(f"{api_base}/v1/redact").mock(
        return_value=httpx.Response(401, json={"error": "bad key"})
    )
    resp = app_client.post(
        "/redact/v1/chat/completions",
        json={
            "model": "gpt-4o-mini",
            "messages": [{"role": "user", "content": "x"}],
        },
        headers={"x-request-id": "req-401"},
    )
    assert resp.status_code == 500
    assert "Invalid Peyeeye API key" in resp.json()["detail"]


@respx.mock
def test_redact_invalid_json_body(app_client):
    resp = app_client.post(
        "/redact/v1/chat/completions",
        content=b"not json",
        headers={
            "Content-Type": "application/json",
            "x-request-id": "req-bad-json",
        },
    )
    assert resp.status_code == 400


# --------------------------------------------------------- /rehydrate endpoint


@respx.mock
def test_rehydrate_round_trip(app_client, api_base):
    """Redact then rehydrate share the request id; placeholders restored."""
    respx.post(f"{api_base}/v1/redact").mock(
        return_value=httpx.Response(
            200,
            json={
                "text": ["hello [EMAIL_0]"],
                "session_id": "ses_round",
            },
        )
    )
    respx.post(f"{api_base}/v1/rehydrate").mock(
        return_value=httpx.Response(
            200, json={"text": "Reach jane@example.com today.", "replaced": 1}
        )
    )
    respx.delete(f"{api_base}/v1/sessions/ses_round").mock(
        return_value=httpx.Response(204)
    )

    redact_resp = app_client.post(
        "/redact/v1/chat/completions",
        json={
            "model": "gpt-4o-mini",
            "messages": [{"role": "user", "content": "hello jane@example.com"}],
        },
        headers={"x-request-id": "req-round"},
    )
    assert redact_resp.status_code == 200

    rehydrate_resp = app_client.post(
        "/rehydrate/v1/chat/completions",
        json={"choices": [{"message": {"content": "Reach [EMAIL_0] today."}}]},
        headers={"x-request-id": "req-round"},
    )
    assert rehydrate_resp.status_code == 200
    body = rehydrate_resp.json()
    assert body["choices"][0]["message"]["content"] == "Reach jane@example.com today."


@respx.mock
def test_rehydrate_no_session_cached_passthrough(app_client):
    raw = {"choices": [{"message": {"content": "no session here"}}]}
    resp = app_client.post(
        "/rehydrate/v1/chat/completions",
        json=raw,
        headers={"x-request-id": "req-uncached"},
    )
    assert resp.status_code == 200
    assert resp.json() == raw


@respx.mock
def test_rehydrate_anthropic_messages(app_client, api_base):
    respx.post(f"{api_base}/v1/redact").mock(
        return_value=httpx.Response(
            200,
            json={"text": ["My ssn is [SSN_0]"], "session_id": "ses_anth"},
        )
    )
    respx.post(f"{api_base}/v1/rehydrate").mock(
        return_value=httpx.Response(
            200, json={"text": "My ssn is 111-22-3333", "replaced": 1}
        )
    )
    respx.delete(f"{api_base}/v1/sessions/ses_anth").mock(
        return_value=httpx.Response(204)
    )

    app_client.post(
        "/redact/v1/messages",
        json={
            "model": "claude-sonnet-4",
            "messages": [{"role": "user", "content": "My ssn is 111-22-3333"}],
        },
        headers={"x-request-id": "req-anth"},
    )
    resp = app_client.post(
        "/rehydrate/v1/messages",
        json={"content": [{"type": "text", "text": "My ssn is [SSN_0]"}]},
        headers={"x-request-id": "req-anth"},
    )
    assert resp.status_code == 200
    body = resp.json()
    assert body["content"][0]["text"] == "My ssn is 111-22-3333"


@respx.mock
def test_rehydrate_sse_passthrough(app_client, api_base):
    """SSE bodies are passed through unchanged (and the session id is kept)."""
    respx.post(f"{api_base}/v1/redact").mock(
        return_value=httpx.Response(
            200, json={"text": ["[EMAIL_0]"], "session_id": "ses_sse"}
        )
    )
    app_client.post(
        "/redact/v1/chat/completions",
        json={
            "model": "gpt-4o-mini",
            "messages": [{"role": "user", "content": "jane@example.com"}],
        },
        headers={"x-request-id": "req-sse"},
    )
    sse_body = b'data: {"choices":[{"delta":{"content":"hi"}}]}\n\n'
    resp = app_client.post(
        "/rehydrate/v1/chat/completions",
        content=sse_body,
        headers={
            "Content-Type": "text/event-stream",
            "x-request-id": "req-sse",
        },
    )
    assert resp.status_code == 200
    assert resp.content == sse_body


@respx.mock
def test_redact_responses_input_string(app_client, api_base):
    respx.post(f"{api_base}/v1/redact").mock(
        return_value=httpx.Response(
            200,
            json={
                "text": ["My email is [EMAIL_0]"],
                "session_id": "ses_resp",
            },
        )
    )
    resp = app_client.post(
        "/redact/v1/responses",
        json={"model": "gpt-4o-mini", "input": "My email is jane@example.com"},
        headers={"x-request-id": "req-resp"},
    )
    assert resp.status_code == 200
    assert resp.json()["input"] == "My email is [EMAIL_0]"


@respx.mock
def test_stateless_session_mode(monkeypatch, api_base):
    """Stateless mode caches the rehydration_key and skips DELETE."""
    monkeypatch.setenv("PEYEEYE_API_KEY", "pk_test_123")
    monkeypatch.setenv("PEYEEYE_SESSION_MODE", "stateless")
    client = PEyeEyeClient()
    test_client = TestClient(create_app(client=client))

    redact_route = respx.post(f"{api_base}/v1/redact").mock(
        return_value=httpx.Response(
            200,
            json={
                "text": ["hello [EMAIL_0]"],
                "rehydration_key": "skey_xyz",
            },
        )
    )
    rehydrate_route = respx.post(f"{api_base}/v1/rehydrate").mock(
        return_value=httpx.Response(
            200, json={"text": "hello jane@example.com", "replaced": 1}
        )
    )
    delete_route = respx.delete(f"{api_base}/v1/sessions/skey_xyz")

    test_client.post(
        "/redact/v1/chat/completions",
        json={
            "model": "gpt-4o-mini",
            "messages": [{"role": "user", "content": "hello jane@example.com"}],
        },
        headers={"x-request-id": "req-stateless"},
    )
    # Inspect the redact request body to confirm session=stateless was sent.
    assert redact_route.called
    sent_body = json.loads(redact_route.calls.last.request.content)
    assert sent_body["session"] == "stateless"

    resp = test_client.post(
        "/rehydrate/v1/chat/completions",
        json={"choices": [{"message": {"content": "hello [EMAIL_0]"}}]},
        headers={"x-request-id": "req-stateless"},
    )
    assert resp.status_code == 200
    assert resp.json()["choices"][0]["message"]["content"] == "hello jane@example.com"
    # Stateless: no DELETE call.
    assert not delete_route.called
    assert rehydrate_route.called
