"""Default config synthesizer for zero-config ``planoai up``.

When the user runs ``planoai up`` in a directory with no ``config.yaml`` /
``plano_config.yaml``, we synthesize a pass-through config that covers the
common LLM providers and auto-wires OTel export to ``localhost:4317`` so
``planoai obs`` works out of the box.

Auth handling:
- If the provider's env var is set, bind ``access_key: $ENV_VAR``.
- Otherwise set ``passthrough_auth: true`` so the client's own Authorization
  header is forwarded. No env var is required to start the proxy.
"""

from __future__ import annotations

import os
from dataclasses import dataclass

DEFAULT_LLM_LISTENER_PORT = 12000
# plano_config validation requires an http:// scheme on the OTLP endpoint.
DEFAULT_OTLP_ENDPOINT = "http://localhost:4317"


@dataclass(frozen=True)
class ProviderDefault:
    name: str
    env_var: str
    base_url: str
    model_pattern: str
    # Only set for providers whose prefix in the model pattern is NOT one of the
    # built-in SUPPORTED_PROVIDERS in cli/planoai/config_generator.py. For
    # built-ins, the validator infers the interface from the model prefix and
    # rejects configs that set this field explicitly.
    provider_interface: str | None = None


# Keep ordering stable so synthesized configs diff cleanly across runs.
PROVIDER_DEFAULTS: list[ProviderDefault] = [
    ProviderDefault(
        name="openai",
        env_var="OPENAI_API_KEY",
        base_url="https://api.openai.com/v1",
        model_pattern="openai/*",
    ),
    ProviderDefault(
        name="anthropic",
        env_var="ANTHROPIC_API_KEY",
        base_url="https://api.anthropic.com/v1",
        model_pattern="anthropic/*",
    ),
    ProviderDefault(
        name="gemini",
        env_var="GEMINI_API_KEY",
        base_url="https://generativelanguage.googleapis.com/v1beta",
        model_pattern="gemini/*",
    ),
    ProviderDefault(
        name="groq",
        env_var="GROQ_API_KEY",
        base_url="https://api.groq.com/openai/v1",
        model_pattern="groq/*",
    ),
    ProviderDefault(
        name="deepseek",
        env_var="DEEPSEEK_API_KEY",
        base_url="https://api.deepseek.com/v1",
        model_pattern="deepseek/*",
    ),
    ProviderDefault(
        name="mistral",
        env_var="MISTRAL_API_KEY",
        base_url="https://api.mistral.ai/v1",
        model_pattern="mistral/*",
    ),
    # DigitalOcean Gradient is a first-class provider post-#889 — the
    # `digitalocean/` model prefix routes to the built-in Envoy cluster, no
    # base_url needed at runtime.
    ProviderDefault(
        name="digitalocean",
        env_var="DO_API_KEY",
        base_url="https://inference.do-ai.run/v1",
        model_pattern="digitalocean/*",
    ),
]


@dataclass
class DetectionResult:
    with_keys: list[ProviderDefault]
    passthrough: list[ProviderDefault]

    @property
    def summary(self) -> str:
        parts = []
        if self.with_keys:
            parts.append("env-keyed: " + ", ".join(p.name for p in self.with_keys))
        if self.passthrough:
            parts.append("pass-through: " + ", ".join(p.name for p in self.passthrough))
        return " | ".join(parts) if parts else "no providers"


def detect_providers(env: dict[str, str] | None = None) -> DetectionResult:
    env = env if env is not None else dict(os.environ)
    with_keys: list[ProviderDefault] = []
    passthrough: list[ProviderDefault] = []
    for p in PROVIDER_DEFAULTS:
        val = env.get(p.env_var)
        if val:
            with_keys.append(p)
        else:
            passthrough.append(p)
    return DetectionResult(with_keys=with_keys, passthrough=passthrough)


def synthesize_default_config(
    env: dict[str, str] | None = None,
    *,
    listener_port: int = DEFAULT_LLM_LISTENER_PORT,
    otel_endpoint: str = DEFAULT_OTLP_ENDPOINT,
) -> dict:
    """Build a pass-through config dict suitable for validation + envoy rendering.

    The returned dict can be dumped to YAML and handed to the existing `planoai up`
    pipeline unchanged.
    """
    detection = detect_providers(env)

    def _entry(p: ProviderDefault, base: dict) -> dict:
        row: dict = {"name": p.name, "model": p.model_pattern, "base_url": p.base_url}
        if p.provider_interface is not None:
            row["provider_interface"] = p.provider_interface
        row.update(base)
        return row

    model_providers: list[dict] = []
    for p in detection.with_keys:
        model_providers.append(_entry(p, {"access_key": f"${p.env_var}"}))
    for p in detection.passthrough:
        model_providers.append(_entry(p, {"passthrough_auth": True}))

    # No explicit `default: true` entry is synthesized: the plano config
    # validator rejects wildcard models as defaults, and brightstaff already
    # registers bare model names as lookup keys during wildcard expansion
    # (crates/common/src/llm_providers.rs), so `{"model": "gpt-4o-mini"}`
    # without a prefix resolves via the openai wildcard without needing
    # `default: true`. See discussion on #890.

    return {
        "version": "v0.4.0",
        "listeners": [
            {
                "name": "llm",
                "type": "model",
                "port": listener_port,
                "address": "0.0.0.0",
            }
        ],
        "model_providers": model_providers,
        "tracing": {
            "random_sampling": 100,
            "opentracing_grpc_endpoint": otel_endpoint,
        },
    }
