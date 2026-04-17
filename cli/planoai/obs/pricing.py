"""DigitalOcean Gradient pricing catalog for the obs console.

Ported loosely from ``crates/brightstaff/src/router/model_metrics.rs::fetch_do_pricing``.
Single-source: one fetch at startup, cached for the life of the process.
"""

from __future__ import annotations

import logging
import threading
from dataclasses import dataclass
from typing import Any

import requests

DEFAULT_PRICING_URL = "https://api.digitalocean.com/v2/gen-ai/models/catalog"
FETCH_TIMEOUT_SECS = 5.0


logger = logging.getLogger(__name__)


@dataclass(frozen=True)
class ModelPrice:
    """Input/output $/token rates. Token counts are multiplied by these."""

    input_per_token_usd: float
    output_per_token_usd: float
    cached_input_per_token_usd: float | None = None


class PricingCatalog:
    """In-memory pricing lookup keyed by model id.

    DO's catalog uses ids like ``openai-gpt-5.4``; Plano's resolved model names
    may arrive as ``do/openai-gpt-5.4`` or bare ``openai-gpt-5.4``. We strip the
    leading provider prefix when looking up.
    """

    def __init__(self, prices: dict[str, ModelPrice] | None = None) -> None:
        self._prices: dict[str, ModelPrice] = prices or {}
        self._lock = threading.Lock()

    def __len__(self) -> int:
        with self._lock:
            return len(self._prices)

    def sample_models(self, n: int = 5) -> list[str]:
        with self._lock:
            return list(self._prices.keys())[:n]

    @classmethod
    def fetch(
        cls,
        url: str = DEFAULT_PRICING_URL,
        api_key: str | None = None,
    ) -> "PricingCatalog":
        """Fetch pricing from DO's catalog endpoint. On failure, returns an
        empty catalog (cost column will be blank).

        The catalog endpoint requires a DigitalOcean Personal Access Token —
        this is *not* the same as the inference ``MODEL_ACCESS_KEY`` used at
        runtime. We check ``DIGITALOCEAN_TOKEN`` first (standard DO CLI env
        var), then ``DO_PAT``, then fall back to ``DO_API_KEY``.
        """
        import os

        headers = {}
        token = (
            api_key
            or os.environ.get("DIGITALOCEAN_TOKEN")
            or os.environ.get("DO_PAT")
            or os.environ.get("DO_API_KEY")
        )
        if token:
            headers["Authorization"] = f"Bearer {token}"

        try:
            resp = requests.get(url, headers=headers, timeout=FETCH_TIMEOUT_SECS)
            resp.raise_for_status()
            data = resp.json()
        except Exception as exc:  # noqa: BLE001 — best-effort; never fatal
            logger.warning(
                "DO pricing fetch failed: %s; cost column will be blank. "
                "Set DIGITALOCEAN_TOKEN with a DO Personal Access Token to "
                "enable cost.",
                exc,
            )
            return cls()

        prices = _parse_do_pricing(data)
        if not prices:
            # Dump the first entry's raw shape so we can see which fields DO
            # actually returned — helps when the catalog adds new fields or
            # the response doesn't match our parser.
            import json as _json

            sample_items = _coerce_items(data)
            sample = sample_items[0] if sample_items else data
            logger.warning(
                "DO pricing response had no parseable entries; cost column "
                "will be blank. Sample entry: %s",
                _json.dumps(sample, default=str)[:400],
            )
        return cls(prices)

    def price_for(self, model_name: str | None) -> ModelPrice | None:
        if not model_name:
            return None
        with self._lock:
            # Try the full name first, then stripped prefix, then lowercased variants.
            for candidate in _model_key_candidates(model_name):
                hit = self._prices.get(candidate)
                if hit is not None:
                    return hit
        return None

    def cost_for_call(self, call: Any) -> float | None:
        """Compute USD cost for an LLMCall. Returns None when pricing is unknown."""
        price = self.price_for(getattr(call, "model", None)) or self.price_for(
            getattr(call, "request_model", None)
        )
        if price is None:
            return None
        prompt = int(getattr(call, "prompt_tokens", 0) or 0)
        completion = int(getattr(call, "completion_tokens", 0) or 0)
        cached = int(getattr(call, "cached_input_tokens", 0) or 0)

        # Cached input tokens are priced separately at the cached rate when known;
        # otherwise they're already counted in prompt tokens at the regular rate.
        fresh_prompt = prompt
        if price.cached_input_per_token_usd is not None and cached:
            fresh_prompt = max(0, prompt - cached)
            cost_cached = cached * price.cached_input_per_token_usd
        else:
            cost_cached = 0.0

        cost = (
            fresh_prompt * price.input_per_token_usd
            + completion * price.output_per_token_usd
            + cost_cached
        )
        return round(cost, 6)


def _model_key_candidates(model_name: str) -> list[str]:
    base = model_name.strip()
    out = [base]
    if "/" in base:
        out.append(base.split("/", 1)[1])
    out.extend([v.lower() for v in list(out)])
    # Dedup while preserving order.
    seen: set[str] = set()
    uniq = []
    for key in out:
        if key not in seen:
            seen.add(key)
            uniq.append(key)
    return uniq


def _parse_do_pricing(data: Any) -> dict[str, ModelPrice]:
    """Parse DO catalog response into a ModelPrice map keyed by model id.

    DO's shape (as of 2026-04):
        {
          "data": [
            {"model_id": "openai-gpt-5.4",
             "pricing": {"input_price_per_million": 5.0,
                         "output_price_per_million": 15.0}},
            ...
          ]
        }

    Older/alternate shapes are also accepted (flat top-level fields, or the
    ``id``/``model``/``name`` key).
    """
    prices: dict[str, ModelPrice] = {}
    items = _coerce_items(data)
    for item in items:
        model_id = (
            item.get("model_id")
            or item.get("id")
            or item.get("model")
            or item.get("name")
        )
        if not model_id:
            continue

        # DO nests rates under `pricing`; try that first, then fall back to
        # top-level fields for alternate response shapes.
        sources = [item]
        if isinstance(item.get("pricing"), dict):
            sources.insert(0, item["pricing"])

        input_rate = _extract_rate_from_sources(
            sources,
            ["input_per_token", "input_token_price", "price_input"],
            ["input_price_per_million", "input_per_million", "input_per_mtok"],
        )
        output_rate = _extract_rate_from_sources(
            sources,
            ["output_per_token", "output_token_price", "price_output"],
            ["output_price_per_million", "output_per_million", "output_per_mtok"],
        )
        cached_rate = _extract_rate_from_sources(
            sources,
            [
                "cached_input_per_token",
                "cached_input_token_price",
                "prompt_cache_read_per_token",
            ],
            [
                "cached_input_price_per_million",
                "cached_input_per_million",
                "cached_input_per_mtok",
            ],
        )

        if input_rate is None or output_rate is None:
            continue
        # Treat 0-rate entries as "unknown" so cost falls back to `—` rather
        # than showing a misleading $0.0000. DO's catalog sometimes omits
        # rates for promo/open-weight models.
        if input_rate == 0 and output_rate == 0:
            continue
        prices[str(model_id)] = ModelPrice(
            input_per_token_usd=input_rate,
            output_per_token_usd=output_rate,
            cached_input_per_token_usd=cached_rate,
        )
    return prices


def _coerce_items(data: Any) -> list[dict]:
    if isinstance(data, list):
        return [x for x in data if isinstance(x, dict)]
    if isinstance(data, dict):
        for key in ("data", "models", "pricing", "items"):
            val = data.get(key)
            if isinstance(val, list):
                return [x for x in val if isinstance(x, dict)]
    return []


def _extract_rate_from_sources(
    sources: list[dict],
    per_token_keys: list[str],
    per_million_keys: list[str],
) -> float | None:
    """Return a per-token rate in USD, or None if unknown.

    Some DO catalog responses put per-token values under a field whose name
    says ``_per_million`` (e.g. ``input_price_per_million: 5E-8`` — that's
    $5e-8 per token, not per million). Heuristic: values < 1 are already
    per-token (real per-million rates are ~0.1 to ~100); values >= 1 are
    treated as per-million and divided by 1,000,000.
    """
    for src in sources:
        for key in per_token_keys:
            if key in src and src[key] is not None:
                try:
                    return float(src[key])
                except (TypeError, ValueError):
                    continue
        for key in per_million_keys:
            if key in src and src[key] is not None:
                try:
                    v = float(src[key])
                except (TypeError, ValueError):
                    continue
                if v >= 1:
                    return v / 1_000_000
                return v
    return None
