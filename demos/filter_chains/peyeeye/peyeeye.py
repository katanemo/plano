"""Peyeeye PII redaction & rehydration filter for Plano filter chains.

Two endpoints, mirroring the pii_anonymizer demo:

  POST /redact/{path:path}     — input filter; redacts PII before the LLM call.
  POST /rehydrate/{path:path}  — output filter; restores PII in the LLM response.

The filter delegates detection and rehydration to the Peyeeye API
(``https://api.peyeeye.ai`` by default). Two session modes are supported:

  * ``stateful`` (default) — Peyeeye holds the token -> value mapping under a
    ``ses_...`` id; rehydrate references the id.
  * ``stateless`` — Peyeeye returns a sealed ``skey_...`` blob; nothing is
    retained server-side.

Behavioral invariants (mirrored from the LiteLLM peyeeye guardrail):

  * Pre-call: redact every text-bearing chunk in the request. If the count of
    returned texts doesn't match the count sent, raise -- never forward
    partially-redacted source.
  * Post-call: pull the cached session id, rehydrate the response, and best-
    effort delete the stateful session.
  * Fail-closed: any unexpected response shape from /v1/redact raises a typed
    error rather than silently passing PII through.

Configuration knobs (env vars):

  * ``PEYEEYE_API_KEY``      — required.
  * ``PEYEEYE_API_BASE``     — defaults to ``https://api.peyeeye.ai``.
  * ``PEYEEYE_LOCALE``       — BCP-47, default ``auto``.
  * ``PEYEEYE_ENTITIES``     — comma-separated entity ids to restrict detection.
  * ``PEYEEYE_SESSION_MODE`` — ``stateful`` (default) or ``stateless``.
"""

from __future__ import annotations

import asyncio
import json
import logging
import os
import threading
import time
from typing import Any, Dict, List, Optional, Tuple

import httpx
from fastapi import FastAPI, HTTPException, Request
from fastapi.responses import JSONResponse, Response

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s - [PEYEEYE] - %(levelname)s - %(message)s",
)
logger = logging.getLogger(__name__)


DEFAULT_API_BASE = "https://api.peyeeye.ai"
SESSION_TTL_SECONDS = 3600
HTTP_TIMEOUT_SECONDS = 15.0


# ----------------------------------------------------------------------- errors


class PEyeEyeAPIError(Exception):
    """Raised when the Peyeeye API returns an error or unexpected payload."""


class PEyeEyeMissingSecrets(Exception):
    """Raised when no Peyeeye API key is configured."""


# ------------------------------------------------------------------- mini cache


class _SessionCache:
    """Tiny in-memory ``request_id -> session_id`` store with TTL."""

    def __init__(self, ttl_seconds: int = SESSION_TTL_SECONDS) -> None:
        self._ttl = ttl_seconds
        self._lock = threading.Lock()
        self._store: Dict[str, Tuple[str, float]] = {}

    def _expire(self) -> None:
        now = time.time()
        expired = [k for k, (_, ts) in self._store.items() if now - ts > self._ttl]
        for k in expired:
            del self._store[k]

    def set(self, key: str, value: str) -> None:
        with self._lock:
            self._expire()
            self._store[key] = (value, time.time())

    def get(self, key: str) -> Optional[str]:
        with self._lock:
            entry = self._store.get(key)
            return entry[0] if entry else None

    def pop(self, key: str) -> Optional[str]:
        with self._lock:
            entry = self._store.pop(key, None)
            return entry[0] if entry else None


# ------------------------------------------------------------------ peyeeye client


class PEyeEyeClient:
    """Async HTTP client for the Peyeeye redact/rehydrate API."""

    def __init__(
        self,
        api_key: Optional[str] = None,
        api_base: Optional[str] = None,
        locale: Optional[str] = None,
        entities: Optional[List[str]] = None,
        session_mode: Optional[str] = None,
    ) -> None:
        key = api_key or os.environ.get("PEYEEYE_API_KEY")
        if not key:
            raise PEyeEyeMissingSecrets(
                "Peyeeye API key missing — set the PEYEEYE_API_KEY env var."
            )
        self.api_key = key
        self.api_base = (
            api_base or os.environ.get("PEYEEYE_API_BASE") or DEFAULT_API_BASE
        ).rstrip("/")
        self.locale = locale or os.environ.get("PEYEEYE_LOCALE") or "auto"
        env_entities = os.environ.get("PEYEEYE_ENTITIES")
        if entities is None and env_entities:
            entities = [e.strip() for e in env_entities.split(",") if e.strip()]
        self.entities = entities or None
        mode = session_mode or os.environ.get("PEYEEYE_SESSION_MODE") or "stateful"
        if mode not in ("stateful", "stateless"):
            raise ValueError(
                f"PEYEEYE_SESSION_MODE must be 'stateful' or 'stateless', got {mode!r}"
            )
        self.session_mode = mode
        self._client: Optional[httpx.AsyncClient] = None

    async def _async_client(self) -> httpx.AsyncClient:
        if self._client is None:
            self._client = httpx.AsyncClient(timeout=HTTP_TIMEOUT_SECONDS)
        return self._client

    def _headers(self) -> Dict[str, str]:
        return {
            "Authorization": f"Bearer {self.api_key}",
            "Content-Type": "application/json",
        }

    async def redact_batch(self, texts: List[str]) -> Tuple[List[str], Optional[str]]:
        """Redact a batch of texts. Returns ``(redacted, session_id_or_skey)``.

        Raises ``PEyeEyeAPIError`` on any non-2xx, timeout, or unexpected shape.
        """
        body: Dict[str, Any] = {"text": texts, "locale": self.locale}
        if self.entities:
            body["entities"] = list(self.entities)
        if self.session_mode == "stateless":
            body["session"] = "stateless"

        payload = await self._post("/v1/redact", body)
        out = payload.get("text")
        if isinstance(out, str):
            redacted = [out]
        elif isinstance(out, list):
            redacted = [str(x) for x in out]
        else:
            raise PEyeEyeAPIError(
                "Peyeeye /v1/redact returned an unexpected response shape; "
                "refusing to forward unredacted text."
            )

        if self.session_mode == "stateless":
            session_id = payload.get("rehydration_key")
        else:
            session_id = payload.get("session_id") or payload.get("session")

        return redacted, session_id

    async def rehydrate(self, text: str, session_id: str) -> str:
        if not text:
            return text
        try:
            payload = await self._post(
                "/v1/rehydrate", {"text": text, "session": session_id}
            )
        except PEyeEyeAPIError as e:
            # Rehydration failures must not corrupt or drop the LLM response;
            # log and fall back to the (already-redacted) text.
            logger.warning("rehydrate failed: %s", e)
            return text
        return payload.get("text", text)

    async def delete_session(self, session_id: str) -> None:
        if not session_id.startswith("ses_"):
            return
        client = await self._async_client()
        try:
            await client.delete(
                f"{self.api_base}/v1/sessions/{session_id}",
                headers=self._headers(),
                timeout=10.0,
            )
        except Exception as e:  # pragma: no cover - best effort
            logger.debug("session cleanup failed: %s", e)

    async def _post(self, path: str, body: Dict[str, Any]) -> Dict[str, Any]:
        client = await self._async_client()
        url = f"{self.api_base}{path}"
        try:
            resp = await client.post(url, headers=self._headers(), json=body)
        except httpx.TimeoutException as e:
            raise PEyeEyeAPIError(f"Peyeeye {path} timed out") from e
        except httpx.HTTPError as e:
            raise PEyeEyeAPIError(f"Peyeeye {path} request failed: {e}") from e
        if resp.status_code == 401:
            raise PEyeEyeMissingSecrets("Invalid Peyeeye API key") from None
        if resp.status_code >= 400:
            raise PEyeEyeAPIError(
                f"Peyeeye {path} returned HTTP {resp.status_code}: {resp.text[:200]}"
            )
        try:
            return resp.json()
        except json.JSONDecodeError as e:
            raise PEyeEyeAPIError(f"Peyeeye {path} returned non-JSON body") from e


# ------------------------------------------------------------- request walkers


def iter_request_texts(body: Dict[str, Any], endpoint: str) -> List[Tuple[str, ...]]:
    """Yield ``("path", ...)`` tuples identifying every user-text chunk.

    Mirrors the litellm pre-call hook: walks ``messages[].content`` (string or
    multimodal text-part list) for chat-style endpoints, and ``input`` for the
    OpenAI Responses API.

    Returns a list of (path-tuple, text) pairs. ``path-tuple`` is consumed by
    ``set_request_text`` to write the redacted value back.
    """
    parts: List[Tuple[Tuple[Any, ...], str]] = []

    if endpoint == "/v1/responses":
        input_val = body.get("input")
        if isinstance(input_val, str) and input_val:
            parts.append((("input",), input_val))
        elif isinstance(input_val, list):
            for i, item in enumerate(input_val):
                if not isinstance(item, dict):
                    continue
                if item.get("role") != "user":
                    continue
                content = item.get("content")
                if isinstance(content, str) and content:
                    parts.append((("input", i, "content"), content))
                elif isinstance(content, list):
                    for j, sub in enumerate(content):
                        if isinstance(sub, dict) and sub.get("type") == "text":
                            text = sub.get("text", "")
                            if text:
                                parts.append((("input", i, "content", j, "text"), text))
        return parts

    # /v1/chat/completions and /v1/messages both use messages[]
    messages = body.get("messages") or []
    for i, msg in enumerate(messages):
        if not isinstance(msg, dict):
            continue
        if msg.get("role") != "user":
            continue
        content = msg.get("content")
        if isinstance(content, str) and content:
            parts.append((("messages", i, "content"), content))
        elif isinstance(content, list):
            for j, sub in enumerate(content):
                if isinstance(sub, dict) and sub.get("type") == "text":
                    text = sub.get("text", "")
                    if text:
                        parts.append((("messages", i, "content", j, "text"), text))
    return parts


def set_request_text(body: Dict[str, Any], path: Tuple[Any, ...], value: str) -> None:
    """Write ``value`` into ``body`` at the given path."""
    cur: Any = body
    for key in path[:-1]:
        cur = cur[key]
    cur[path[-1]] = value


def iter_response_texts(
    body: Dict[str, Any], endpoint: str
) -> List[Tuple[Tuple[Any, ...], str]]:
    """Yield ``(path, text)`` for every text chunk in an LLM response body."""
    parts: List[Tuple[Tuple[Any, ...], str]] = []
    if endpoint == "/v1/messages":
        content = body.get("content")
        if isinstance(content, list):
            for j, sub in enumerate(content):
                if isinstance(sub, dict) and sub.get("type") == "text":
                    text = sub.get("text", "")
                    if text:
                        parts.append((("content", j, "text"), text))
        return parts

    # /v1/chat/completions, /v1/responses (synchronous), and similar
    choices = body.get("choices")
    if isinstance(choices, list):
        for i, choice in enumerate(choices):
            if not isinstance(choice, dict):
                continue
            message = choice.get("message")
            if isinstance(message, dict):
                content = message.get("content")
                if isinstance(content, str) and content:
                    parts.append((("choices", i, "message", "content"), content))
    # OpenAI Responses API: top-level "output" array
    output = body.get("output")
    if isinstance(output, list):
        for i, item in enumerate(output):
            if not isinstance(item, dict):
                continue
            content = item.get("content")
            if isinstance(content, list):
                for j, sub in enumerate(content):
                    if isinstance(sub, dict) and sub.get("type") in (
                        "text",
                        "output_text",
                    ):
                        text = sub.get("text", "")
                        if text:
                            parts.append((("output", i, "content", j, "text"), text))
    return parts


# -------------------------------------------------------------------- FastAPI


def create_app(client: Optional[PEyeEyeClient] = None) -> FastAPI:
    """Build the FastAPI app. ``client`` is overridable for tests."""
    app = FastAPI(title="Peyeeye PII filter", version="1.0.0")
    cache = _SessionCache()

    def _client() -> PEyeEyeClient:
        nonlocal client
        if client is None:
            client = PEyeEyeClient()
        return client

    @app.post("/redact/{path:path}")
    async def redact(path: str, request: Request) -> Response:
        endpoint = f"/{path}"
        request_id = request.headers.get("x-request-id", "unknown")
        try:
            body = await request.json()
        except json.JSONDecodeError:
            raise HTTPException(status_code=400, detail="invalid JSON body")

        text_parts = iter_request_texts(body, endpoint)
        if not text_parts:
            logger.info("request_id=%s no text to redact", request_id)
            return JSONResponse(content=body)

        texts = [t for _, t in text_parts]
        try:
            redacted, session_id = await _client().redact_batch(texts)
        except PEyeEyeMissingSecrets as e:
            logger.error("request_id=%s missing secrets: %s", request_id, e)
            raise HTTPException(status_code=500, detail=str(e))
        except PEyeEyeAPIError as e:
            # Fail-closed: do not pass unredacted text through.
            logger.error("request_id=%s redact failed: %s", request_id, e)
            raise HTTPException(status_code=502, detail=str(e))

        # Length-guard: must match exactly. The API contract is one-to-one.
        if len(redacted) != len(text_parts):
            logger.error(
                "request_id=%s length mismatch: sent=%d got=%d",
                request_id,
                len(text_parts),
                len(redacted),
            )
            raise HTTPException(
                status_code=502,
                detail=(
                    f"Peyeeye /v1/redact returned {len(redacted)} texts for "
                    f"{len(text_parts)} inputs; refusing to forward partially-"
                    "redacted data"
                ),
            )

        for (path_tuple, _), value in zip(text_parts, redacted):
            set_request_text(body, path_tuple, value)

        if session_id:
            cache.set(request_id, session_id)
            logger.info(
                "request_id=%s redacted %d chunk(s); cached session",
                request_id,
                len(text_parts),
            )
        else:
            logger.info(
                "request_id=%s redacted %d chunk(s); no session id returned",
                request_id,
                len(text_parts),
            )

        return JSONResponse(content=body)

    @app.post("/rehydrate/{path:path}")
    async def rehydrate(path: str, request: Request) -> Response:
        endpoint = f"/{path}"
        request_id = request.headers.get("x-request-id", "unknown")
        raw = await request.body()

        session_id = cache.pop(request_id)
        if not session_id:
            logger.info(
                "request_id=%s no session cached, passing response through",
                request_id,
            )
            return Response(content=raw, media_type="application/json")

        # Streaming SSE: not supported for stateful rehydration in this demo.
        # Plano sends raw chunks, but rehydration needs the full token to look
        # up the original; we pass through and rely on a non-streaming flow.
        body_str = raw.decode("utf-8", errors="replace")
        if body_str.lstrip().startswith("data:") or "data: " in body_str[:32]:
            logger.warning(
                "request_id=%s SSE not supported; passing through "
                "(use non-streaming for now)",
                request_id,
            )
            # Don't lose the session id on the way back if SSE.
            cache.set(request_id, session_id)
            return Response(content=raw, media_type="text/event-stream")

        try:
            body = json.loads(body_str)
        except json.JSONDecodeError:
            logger.warning(
                "request_id=%s response is not JSON; passing through", request_id
            )
            return Response(content=raw, media_type="application/json")

        text_parts = iter_response_texts(body, endpoint)
        if not text_parts:
            logger.info("request_id=%s no response text to rehydrate", request_id)
            await _maybe_delete_session(_client(), session_id)
            return JSONResponse(content=body)

        # Run rehydrate calls concurrently — each returns the original text,
        # falling back to the redacted value on rehydrate error.
        tasks = [_client().rehydrate(text, session_id) for _, text in text_parts]
        restored = await asyncio.gather(*tasks)
        for (path_tuple, _), value in zip(text_parts, restored):
            set_request_text(body, path_tuple, value)

        await _maybe_delete_session(_client(), session_id)
        logger.info("request_id=%s rehydrated %d chunk(s)", request_id, len(text_parts))
        return JSONResponse(content=body)

    @app.get("/health")
    async def health() -> Dict[str, str]:
        return {"status": "healthy"}

    return app


async def _maybe_delete_session(client: PEyeEyeClient, session_id: str) -> None:
    """Best-effort delete of a stateful session id."""
    if client.session_mode == "stateful":
        try:
            await client.delete_session(session_id)
        except Exception:  # pragma: no cover - best effort
            pass


# Default ASGI app — used by ``uvicorn peyeeye:app``.
# Lazily resolves the client so tests can construct ``create_app(client=...)``
# without requiring PEYEEYE_API_KEY in the environment.
app = create_app()
