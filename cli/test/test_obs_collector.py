import time
from datetime import datetime, timezone
from types import SimpleNamespace
from unittest.mock import MagicMock

import pytest

from planoai.obs.collector import LLMCall, LLMCallStore, span_to_llm_call


def _mk_attr(key: str, value):
    v = MagicMock()
    if isinstance(value, bool):
        v.WhichOneof.return_value = "bool_value"
        v.bool_value = value
    elif isinstance(value, int):
        v.WhichOneof.return_value = "int_value"
        v.int_value = value
    elif isinstance(value, float):
        v.WhichOneof.return_value = "double_value"
        v.double_value = value
    else:
        v.WhichOneof.return_value = "string_value"
        v.string_value = str(value)
    kv = MagicMock()
    kv.key = key
    kv.value = v
    return kv


def _mk_span(
    attrs: dict, start_ns: int | None = None, span_id_hex: str = "ab"
) -> MagicMock:
    span = MagicMock()
    span.attributes = [_mk_attr(k, v) for k, v in attrs.items()]
    span.start_time_unix_nano = start_ns or int(time.time() * 1_000_000_000)
    span.span_id.hex.return_value = span_id_hex
    return span


def test_span_without_llm_model_is_ignored():
    span = _mk_span({"http.method": "POST"})
    assert span_to_llm_call(span, "plano(llm)") is None


def test_span_with_full_llm_attrs_produces_call():
    span = _mk_span(
        {
            "llm.model": "openai-gpt-5.4",
            "model.requested": "router:software-engineering",
            "plano.session_id": "sess-abc",
            "plano.route.name": "software-engineering",
            "llm.is_streaming": False,
            "llm.duration_ms": 1234,
            "llm.time_to_first_token": 210,
            "llm.usage.prompt_tokens": 100,
            "llm.usage.completion_tokens": 50,
            "llm.usage.total_tokens": 150,
            "llm.usage.cached_input_tokens": 30,
            "llm.usage.cache_creation_tokens": 5,
            "llm.usage.reasoning_tokens": 200,
            "http.status_code": 200,
            "request_id": "req-42",
        }
    )
    call = span_to_llm_call(span, "plano(llm)")
    assert call is not None
    assert call.request_id == "req-42"
    assert call.model == "openai-gpt-5.4"
    assert call.request_model == "router:software-engineering"
    assert call.session_id == "sess-abc"
    assert call.route_name == "software-engineering"
    assert call.is_streaming is False
    assert call.duration_ms == 1234.0
    assert call.ttft_ms == 210.0
    assert call.prompt_tokens == 100
    assert call.completion_tokens == 50
    assert call.total_tokens == 150
    assert call.cached_input_tokens == 30
    assert call.cache_creation_tokens == 5
    assert call.reasoning_tokens == 200
    assert call.status_code == 200


def test_pricing_lookup_attaches_cost():
    class StubPricing:
        def cost_for_call(self, call):
            # Simple: 2 * prompt + 3 * completion, in cents
            return 0.02 * (call.prompt_tokens or 0) + 0.03 * (
                call.completion_tokens or 0
            )

    span = _mk_span(
        {
            "llm.model": "do/openai-gpt-5.4",
            "llm.usage.prompt_tokens": 10,
            "llm.usage.completion_tokens": 2,
        }
    )
    call = span_to_llm_call(span, "plano(llm)", pricing=StubPricing())
    assert call is not None
    assert call.cost_usd == pytest.approx(0.26)


def test_tpt_and_tokens_per_sec_derived():
    call = LLMCall(
        request_id="x",
        timestamp=datetime.now(tz=timezone.utc),
        model="m",
        duration_ms=1000,
        ttft_ms=200,
        completion_tokens=80,
    )
    # (1000 - 200) / 80 = 10ms per token => 100 tokens/sec
    assert call.tpt_ms == 10.0
    assert call.tokens_per_sec == 100.0


def test_tpt_returns_none_when_no_completion_tokens():
    call = LLMCall(
        request_id="x",
        timestamp=datetime.now(tz=timezone.utc),
        model="m",
        duration_ms=1000,
        ttft_ms=200,
        completion_tokens=0,
    )
    assert call.tpt_ms is None
    assert call.tokens_per_sec is None


def test_store_evicts_fifo_at_capacity():
    store = LLMCallStore(capacity=3)
    now = datetime.now(tz=timezone.utc)
    for i in range(5):
        store.add(
            LLMCall(
                request_id=f"r{i}",
                timestamp=now,
                model="m",
            )
        )
    snap = store.snapshot()
    assert len(snap) == 3
    assert [c.request_id for c in snap] == ["r2", "r3", "r4"]
