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


def test_anthropic_aliases_match_plano_emitted_names():
    """DO publishes 'anthropic-claude-opus-4.7' and 'anthropic-claude-haiku-4.5';
    Plano emits 'claude-opus-4-7' and 'claude-haiku-4-5-20251001'. Aliases
    registered at parse time should bridge the gap."""
    from planoai.obs.pricing import _parse_do_pricing

    sample = {
        "data": [
            {
                "model_id": "anthropic-claude-opus-4.7",
                "pricing": {
                    "input_price_per_million": 15.0,
                    "output_price_per_million": 75.0,
                },
            },
            {
                "model_id": "anthropic-claude-haiku-4.5",
                "pricing": {
                    "input_price_per_million": 1.0,
                    "output_price_per_million": 5.0,
                },
            },
            {
                "model_id": "anthropic-claude-4.6-sonnet",
                "pricing": {
                    "input_price_per_million": 3.0,
                    "output_price_per_million": 15.0,
                },
            },
        ]
    }
    catalog = PricingCatalog(_parse_do_pricing(sample))
    # Family-last shapes Plano emits.
    assert catalog.price_for("claude-opus-4-7") is not None
    assert catalog.price_for("claude-haiku-4-5") is not None
    # Date-suffixed name (Anthropic API style).
    assert catalog.price_for("claude-haiku-4-5-20251001") is not None
    # Word-order swap: DO has 'claude-4.6-sonnet', Plano emits 'claude-sonnet-4-6'.
    assert catalog.price_for("claude-sonnet-4-6") is not None
    # Original DO ids still resolve.
    assert catalog.price_for("anthropic-claude-opus-4.7") is not None


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


_MODELS_DEV_SAMPLE = {
    "anthropic": {
        "id": "anthropic",
        "models": {
            "claude-opus-4-5": {
                "id": "claude-opus-4-5",
                "cost": {"input": 5, "output": 25, "cache_read": 0.5},
            }
        },
    },
    "groq": {
        "id": "groq",
        "models": {
            "llama-3.3-70b-versatile": {
                "id": "llama-3.3-70b-versatile",
                "cost": {"input": 0.59, "output": 0.79},
            },
            # No cost block → skipped.
            "whisper-large-v3-turbo": {"id": "whisper-large-v3-turbo"},
        },
    },
}


def test_parse_models_dev_composes_provider_keys_and_per_token_rates():
    from planoai.obs.pricing import _parse_models_dev_pricing

    prices = _parse_models_dev_pricing(_MODELS_DEV_SAMPLE)

    # models.dev cost values are per-million → divided by 1e6.
    opus = prices["anthropic/claude-opus-4-5"]
    assert opus.input_per_token_usd == 5 / 1_000_000
    assert opus.output_per_token_usd == 25 / 1_000_000
    assert opus.cached_input_per_token_usd == 0.5 / 1_000_000

    # Composite provider/model keys match Plano's routing names.
    assert "groq/llama-3.3-70b-versatile" in prices
    # Bare model id registered as a fallback.
    assert "llama-3.3-70b-versatile" in prices
    # Models without a cost block are skipped.
    assert "groq/whisper-large-v3-turbo" not in prices


def test_models_dev_catalog_cost_computation():
    from planoai.obs.pricing import PricingCatalog, _parse_models_dev_pricing

    catalog = PricingCatalog(_parse_models_dev_pricing(_MODELS_DEV_SAMPLE))
    # 1000 input @ 5e-6 = 0.005; 500 output @ 25e-6 = 0.0125
    cost = catalog.cost_for_call(_call("anthropic/claude-opus-4-5", 1000, 500))
    assert cost == round(0.005 + 0.0125, 6)


def test_models_dev_skips_zero_rate_entries():
    from planoai.obs.pricing import _parse_models_dev_pricing

    sample = {
        "free": {
            "models": {
                "promo-model": {"cost": {"input": 0, "output": 0}},
            }
        }
    }
    assert _parse_models_dev_pricing(sample) == {}
