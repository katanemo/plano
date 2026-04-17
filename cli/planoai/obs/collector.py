"""In-memory collector for LLM calls, fed by OTLP/gRPC spans from brightstaff."""

from __future__ import annotations

import threading
from collections import deque
from concurrent import futures
from dataclasses import dataclass, field
from datetime import datetime, timezone
from typing import Any, Iterable

import grpc
from opentelemetry.proto.collector.trace.v1 import (
    trace_service_pb2,
    trace_service_pb2_grpc,
)

DEFAULT_GRPC_PORT = 4317
DEFAULT_CAPACITY = 1000


@dataclass
class LLMCall:
    """One LLM call as reconstructed from a brightstaff LLM span.

    Fields default to ``None`` when the underlying span attribute was absent.
    """

    request_id: str
    timestamp: datetime
    model: str
    provider: str | None = None
    request_model: str | None = None
    session_id: str | None = None
    route_name: str | None = None
    is_streaming: bool | None = None
    status_code: int | None = None
    prompt_tokens: int | None = None
    completion_tokens: int | None = None
    total_tokens: int | None = None
    cached_input_tokens: int | None = None
    cache_creation_tokens: int | None = None
    reasoning_tokens: int | None = None
    ttft_ms: float | None = None
    duration_ms: float | None = None
    routing_strategy: str | None = None
    routing_reason: str | None = None
    cost_usd: float | None = None

    @property
    def tpt_ms(self) -> float | None:
        if self.duration_ms is None or self.completion_tokens in (None, 0):
            return None
        ttft = self.ttft_ms or 0.0
        generate_ms = max(0.0, self.duration_ms - ttft)
        if generate_ms <= 0:
            return None
        return generate_ms / self.completion_tokens

    @property
    def tokens_per_sec(self) -> float | None:
        tpt = self.tpt_ms
        if tpt is None or tpt <= 0:
            return None
        return 1000.0 / tpt


class LLMCallStore:
    """Thread-safe ring buffer of recent LLM calls."""

    def __init__(self, capacity: int = DEFAULT_CAPACITY) -> None:
        self._capacity = capacity
        self._calls: deque[LLMCall] = deque(maxlen=capacity)
        self._lock = threading.Lock()

    @property
    def capacity(self) -> int:
        return self._capacity

    def add(self, call: LLMCall) -> None:
        with self._lock:
            self._calls.append(call)

    def clear(self) -> None:
        with self._lock:
            self._calls.clear()

    def snapshot(self) -> list[LLMCall]:
        with self._lock:
            return list(self._calls)

    def __len__(self) -> int:
        with self._lock:
            return len(self._calls)


# Span attribute keys used below are the canonical OTel / Plano keys emitted by
# brightstaff — see crates/brightstaff/src/tracing/constants.rs for the source
# of truth.


def _anyvalue_to_python(value: Any) -> Any:  # AnyValue from OTLP
    kind = value.WhichOneof("value")
    if kind == "string_value":
        return value.string_value
    if kind == "bool_value":
        return value.bool_value
    if kind == "int_value":
        return value.int_value
    if kind == "double_value":
        return value.double_value
    return None


def _attrs_to_dict(attrs: Iterable[Any]) -> dict[str, Any]:
    out: dict[str, Any] = {}
    for kv in attrs:
        py = _anyvalue_to_python(kv.value)
        if py is not None:
            out[kv.key] = py
    return out


def _maybe_int(value: Any) -> int | None:
    if value is None:
        return None
    try:
        return int(value)
    except (TypeError, ValueError):
        return None


def _maybe_float(value: Any) -> float | None:
    if value is None:
        return None
    try:
        return float(value)
    except (TypeError, ValueError):
        return None


def span_to_llm_call(
    span: Any, service_name: str, pricing: Any | None = None
) -> LLMCall | None:
    """Convert an OTLP span into an LLMCall, or return None if it isn't one.

    A span is considered an LLM call iff it carries the ``llm.model`` attribute.
    """
    attrs = _attrs_to_dict(span.attributes)
    model = attrs.get("llm.model")
    if not model:
        return None

    # Prefer explicit span attributes; fall back to likely aliases.
    request_id = next(
        (
            str(attrs[key])
            for key in ("request_id", "http.request_id")
            if key in attrs and attrs[key] is not None
        ),
        span.span_id.hex() if span.span_id else "",
    )
    start_ns = span.start_time_unix_nano or 0
    ts = (
        datetime.fromtimestamp(start_ns / 1_000_000_000, tz=timezone.utc).astimezone()
        if start_ns
        else datetime.now().astimezone()
    )

    call = LLMCall(
        request_id=str(request_id),
        timestamp=ts,
        model=str(model),
        provider=(
            str(attrs["llm.provider"]) if "llm.provider" in attrs else service_name
        ),
        request_model=(
            str(attrs["model.requested"]) if "model.requested" in attrs else None
        ),
        session_id=(
            str(attrs["plano.session_id"]) if "plano.session_id" in attrs else None
        ),
        route_name=(
            str(attrs["plano.route.name"]) if "plano.route.name" in attrs else None
        ),
        is_streaming=(
            bool(attrs["llm.is_streaming"]) if "llm.is_streaming" in attrs else None
        ),
        status_code=_maybe_int(attrs.get("http.status_code")),
        prompt_tokens=_maybe_int(attrs.get("llm.usage.prompt_tokens")),
        completion_tokens=_maybe_int(attrs.get("llm.usage.completion_tokens")),
        total_tokens=_maybe_int(attrs.get("llm.usage.total_tokens")),
        cached_input_tokens=_maybe_int(attrs.get("llm.usage.cached_input_tokens")),
        cache_creation_tokens=_maybe_int(attrs.get("llm.usage.cache_creation_tokens")),
        reasoning_tokens=_maybe_int(attrs.get("llm.usage.reasoning_tokens")),
        ttft_ms=_maybe_float(attrs.get("llm.time_to_first_token")),
        duration_ms=_maybe_float(attrs.get("llm.duration_ms")),
        routing_strategy=(
            str(attrs["routing.strategy"]) if "routing.strategy" in attrs else None
        ),
        routing_reason=(
            str(attrs["routing.selection_reason"])
            if "routing.selection_reason" in attrs
            else None
        ),
    )

    if pricing is not None:
        call.cost_usd = pricing.cost_for_call(call)

    return call


class _ObsServicer(trace_service_pb2_grpc.TraceServiceServicer):
    def __init__(self, store: LLMCallStore, pricing: Any | None) -> None:
        self._store = store
        self._pricing = pricing

    def Export(self, request, context):  # noqa: N802 — gRPC generated name
        for resource_spans in request.resource_spans:
            service_name = "unknown"
            for attr in resource_spans.resource.attributes:
                if attr.key == "service.name":
                    val = _anyvalue_to_python(attr.value)
                    if val is not None:
                        service_name = str(val)
                    break
            for scope_spans in resource_spans.scope_spans:
                for span in scope_spans.spans:
                    call = span_to_llm_call(span, service_name, self._pricing)
                    if call is not None:
                        self._store.add(call)
        return trace_service_pb2.ExportTraceServiceResponse()


@dataclass
class ObsCollector:
    """Owns the OTLP/gRPC server and the in-memory LLMCall ring buffer."""

    store: LLMCallStore = field(default_factory=LLMCallStore)
    pricing: Any | None = None
    host: str = "0.0.0.0"
    port: int = DEFAULT_GRPC_PORT
    _server: grpc.Server | None = field(default=None, init=False, repr=False)

    def start(self) -> None:
        if self._server is not None:
            return
        server = grpc.server(futures.ThreadPoolExecutor(max_workers=4))
        trace_service_pb2_grpc.add_TraceServiceServicer_to_server(
            _ObsServicer(self.store, self.pricing), server
        )
        address = f"{self.host}:{self.port}"
        bound = server.add_insecure_port(address)
        if bound == 0:
            raise OSError(
                f"Failed to bind OTLP listener on {address}: port already in use. "
                "Stop tracing via `planoai trace down` or pick another port with --port."
            )
        server.start()
        self._server = server

    def stop(self, grace: float = 2.0) -> None:
        if self._server is not None:
            self._server.stop(grace)
            self._server = None
