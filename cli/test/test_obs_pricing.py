from datetime import datetime, timezone

from planoai.obs.collector import LLMCall
from planoai.obs.pricing import ModelPrice, PricingCatalog


def _call(model: str, prompt: int, completion: int, cached: int = 0) -> LLMCall:
    return LLMCall(
        request_id="r",
        timestamp=datetime.now(tz=timezone.utc),
        model=model,
        prompt_tokens=prompt,
        completion_tokens=completion,
        cached_input_tokens=cached,
    )


def test_lookup_matches_bare_and_prefixed():
    prices = {
        "openai-gpt-5.4": ModelPrice(
            input_per_token_usd=0.000001, output_per_token_usd=0.000002
        )
    }
    catalog = PricingCatalog(prices)
    assert catalog.price_for("openai-gpt-5.4") is not None
    # do/openai-gpt-5.4 should resolve after stripping the provider prefix.
    assert catalog.price_for("do/openai-gpt-5.4") is not None
    assert catalog.price_for("unknown-model") is None


def test_cost_computation_without_cache():
    prices = {
        "m": ModelPrice(input_per_token_usd=0.000001, output_per_token_usd=0.000002)
    }
    cost = PricingCatalog(prices).cost_for_call(_call("m", 1000, 500))
    assert cost == 0.002  # 1000 * 1e-6 + 500 * 2e-6


def test_cost_computation_with_cached_discount():
    prices = {
        "m": ModelPrice(
            input_per_token_usd=0.000001,
            output_per_token_usd=0.000002,
            cached_input_per_token_usd=0.0000001,
        )
    }
    # 800 fresh @ 1e-6 = 8e-4; 200 cached @ 1e-7 = 2e-5; 500 out @ 2e-6 = 1e-3
    cost = PricingCatalog(prices).cost_for_call(_call("m", 1000, 500, cached=200))
    assert cost == round(0.0008 + 0.00002 + 0.001, 6)


def test_empty_catalog_returns_none():
    assert PricingCatalog().cost_for_call(_call("m", 100, 50)) is None


def test_parse_do_catalog_treats_small_values_as_per_token():
    """DO's real catalog uses per-token values under the `_per_million` key
    (e.g. 5E-8 for GPT-oss-20b). We treat values < 1 as already per-token."""
    from planoai.obs.pricing import _parse_do_pricing

    sample = {
        "data": [
            {
                "model_id": "openai-gpt-oss-20b",
                "pricing": {
                    "input_price_per_million": 5e-8,
                    "output_price_per_million": 4.5e-7,
                },
            },
            {
                "model_id": "openai-gpt-oss-120b",
                "pricing": {
                    "input_price_per_million": 1e-7,
                    "output_price_per_million": 7e-7,
                },
            },
        ]
    }
    prices = _parse_do_pricing(sample)
    # Values < 1 are assumed to already be per-token — no extra division.
    assert prices["openai-gpt-oss-20b"].input_per_token_usd == 5e-8
    assert prices["openai-gpt-oss-20b"].output_per_token_usd == 4.5e-7
    assert prices["openai-gpt-oss-120b"].input_per_token_usd == 1e-7


def test_parse_do_catalog_divides_large_values_as_per_million():
    """A provider that genuinely reports $5-per-million in that field gets divided."""
    from planoai.obs.pricing import _parse_do_pricing

    sample = {
        "data": [
            {
                "model_id": "mystery-model",
                "pricing": {
                    "input_price_per_million": 5.0,  # > 1 → treated as per-million
                    "output_price_per_million": 15.0,
                },
            },
        ]
    }
    prices = _parse_do_pricing(sample)
    assert prices["mystery-model"].input_per_token_usd == 5.0 / 1_000_000
    assert prices["mystery-model"].output_per_token_usd == 15.0 / 1_000_000
