from pathlib import Path

import jsonschema
import yaml

from planoai.defaults import (
    PROVIDER_DEFAULTS,
    detect_providers,
    synthesize_default_config,
)

_SCHEMA_PATH = Path(__file__).parents[2] / "config" / "plano_config_schema.yaml"


def _schema() -> dict:
    return yaml.safe_load(_SCHEMA_PATH.read_text())


def test_zero_env_vars_produces_pure_passthrough():
    cfg = synthesize_default_config(env={})
    assert cfg["version"] == "v0.4.0"
    assert cfg["listeners"][0]["port"] == 12000
    for provider in cfg["model_providers"]:
        assert provider.get("passthrough_auth") is True
        assert "access_key" not in provider
        # No provider should be marked default in pure pass-through mode.
        assert provider.get("default") is not True
    # All known providers should be listed.
    names = {p["name"] for p in cfg["model_providers"]}
    assert "digitalocean" in names
    assert "vercel" in names
    assert "openrouter" in names
    assert "openai" in names
    assert "anthropic" in names


def test_env_keys_promote_providers_to_env_keyed():
    cfg = synthesize_default_config(
        env={"OPENAI_API_KEY": "sk-1", "DO_API_KEY": "do-1"}
    )
    by_name = {p["name"]: p for p in cfg["model_providers"]}
    assert by_name["openai"].get("access_key") == "$OPENAI_API_KEY"
    assert by_name["openai"].get("passthrough_auth") is None
    assert by_name["digitalocean"].get("access_key") == "$DO_API_KEY"
    # Unset env keys remain pass-through.
    assert by_name["anthropic"].get("passthrough_auth") is True


def test_no_default_is_synthesized():
    # Bare model names resolve via brightstaff's wildcard expansion registering
    # bare keys, so the synthesizer intentionally never sets `default: true`.
    cfg = synthesize_default_config(
        env={"OPENAI_API_KEY": "sk-1", "ANTHROPIC_API_KEY": "a-1"}
    )
    assert not any(p.get("default") is True for p in cfg["model_providers"])


def test_listener_port_is_configurable():
    cfg = synthesize_default_config(env={}, listener_port=11000)
    assert cfg["listeners"][0]["port"] == 11000


def test_detection_summary_strings():
    det = detect_providers(env={"OPENAI_API_KEY": "sk", "DO_API_KEY": "d"})
    summary = det.summary
    assert "env-keyed" in summary and "openai" in summary and "digitalocean" in summary
    assert "pass-through" in summary


def test_tracing_block_points_at_local_console():
    cfg = synthesize_default_config(env={})
    tracing = cfg["tracing"]
    assert tracing["opentracing_grpc_endpoint"] == "http://localhost:4317"
    # random_sampling is a percentage in the plano config — 100 = every span.
    assert tracing["random_sampling"] == 100


def test_synthesized_config_validates_against_schema():
    cfg = synthesize_default_config(env={"OPENAI_API_KEY": "sk"})
    jsonschema.validate(cfg, _schema())


def test_provider_defaults_digitalocean_is_configured():
    by_name = {p.name: p for p in PROVIDER_DEFAULTS}
    assert "digitalocean" in by_name
    assert by_name["digitalocean"].env_var == "DO_API_KEY"
    assert by_name["digitalocean"].base_url == "https://inference.do-ai.run/v1"
    assert by_name["digitalocean"].model_pattern == "digitalocean/*"


def test_provider_defaults_vercel_is_configured():
    by_name = {p.name: p for p in PROVIDER_DEFAULTS}
    assert "vercel" in by_name
    assert by_name["vercel"].env_var == "AI_GATEWAY_API_KEY"
    assert by_name["vercel"].base_url == "https://ai-gateway.vercel.sh/v1"
    assert by_name["vercel"].model_pattern == "vercel/*"


def test_provider_defaults_openrouter_is_configured():
    by_name = {p.name: p for p in PROVIDER_DEFAULTS}
    assert "openrouter" in by_name
    assert by_name["openrouter"].env_var == "OPENROUTER_API_KEY"
    assert by_name["openrouter"].base_url == "https://openrouter.ai/api/v1"
    assert by_name["openrouter"].model_pattern == "openrouter/*"


def test_openrouter_env_key_promotes_to_env_keyed():
    cfg = synthesize_default_config(env={"OPENROUTER_API_KEY": "or-1"})
    by_name = {p["name"]: p for p in cfg["model_providers"]}
    assert by_name["openrouter"].get("access_key") == "$OPENROUTER_API_KEY"
    assert by_name["openrouter"].get("passthrough_auth") is None
