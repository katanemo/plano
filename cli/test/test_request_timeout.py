"""Legacy request budgets use the real schema and Envoy template."""

from pathlib import Path

import pytest
import yaml
from jsonschema import ValidationError

from planoai.config_generator import validate_and_render_schema, validate_prompt_config

CONFIG_DIR = Path(__file__).resolve().parents[2] / "config"


def render_config(tmp_path, monkeypatch, *, request_timeout=None, listeners=None):
    config = {
        "version": "v0.4.0",
        "model_providers": [{"model": "openai/test", "access_key": "$TEST_KEY"}],
        "agents": [{"id": "worker", "url": "http://127.0.0.1:18080"}],
        "listeners": listeners
        or [
            {"type": "prompt", "name": "prompt", "port": 10000},
            {"type": "model", "name": "model", "port": 12000},
            {
                "type": "agent",
                "name": "worker",
                "port": 13000,
                "agents": [{"id": "worker", "description": "Local test agent"}],
            },
        ],
    }
    if request_timeout is not None:
        config["request_timeout"] = request_timeout
    source = tmp_path / "plano.yaml"
    source.write_text(yaml.safe_dump(config))
    envoy_path = tmp_path / "envoy.yaml"
    rendered_path = tmp_path / "rendered.yaml"
    for key, value in {
        "PLANO_CONFIG_FILE": source,
        "PLANO_CONFIG_SCHEMA_FILE": CONFIG_DIR / "plano_config_schema.yaml",
        "TEMPLATE_ROOT": CONFIG_DIR,
        "ENVOY_CONFIG_TEMPLATE_FILE": "envoy.template.yaml",
        "PLANO_CONFIG_FILE_RENDERED": rendered_path,
        "ENVOY_CONFIG_FILE_RENDERED": envoy_path,
    }.items():
        monkeypatch.setenv(key, str(value))
    validate_and_render_schema()
    return yaml.safe_load(envoy_path.read_text()), yaml.safe_load(
        rendered_path.read_text()
    )


def route_timeouts(envoy):
    result = {}
    for listener in envoy["static_resources"]["listeners"]:
        manager = listener["filter_chains"][0]["filters"][0]["typed_config"]
        result[listener["name"]] = [
            route["route"]["timeout"]
            for host in manager["route_config"]["virtual_hosts"]
            for route in host["routes"]
            if "route" in route
        ]
    return result


@pytest.mark.parametrize(
    "value,expected", [(None, "120s"), ("40s", "40s"), ("600s", "600s")]
)
def test_legacy_budget_reaches_wasm_and_prompt_route_only(
    tmp_path, monkeypatch, value, expected
):
    envoy, config = render_config(tmp_path, monkeypatch, request_timeout=value)
    assert config["request_timeout"] == expected
    timeouts = route_timeouts(envoy)
    assert timeouts["ingress_traffic"] == [expected]
    assert set(timeouts["outbound_api_traffic"]) == {"300s"}
    assert timeouts["worker"] == ["30s"]
    assert set(timeouts["egress_traffic"]) == {"300s"}
    for listener in envoy["static_resources"]["listeners"]:
        manager = listener["filter_chains"][0]["filters"][0]["typed_config"]
        for http_filter in manager["http_filters"]:
            if http_filter["name"] == "envoy.filters.http.wasm_prompt":
                wasm_config = http_filter["typed_config"]["value"]["config"][
                    "configuration"
                ]["value"]
                assert yaml.safe_load(wasm_config)["request_timeout"] == expected


@pytest.mark.parametrize("legacy", [False, True])
def test_explicit_prompt_listener_timeout_stays_an_outer_cap(
    tmp_path, monkeypatch, legacy
):
    listeners = (
        {"ingress_traffic": {"port": 10000, "timeout": "1s"}}
        if legacy
        else [{"type": "prompt", "name": "prompt", "port": 10000, "timeout": "1s"}]
    )
    envoy, config = render_config(
        tmp_path, monkeypatch, request_timeout="40s", listeners=listeners
    )
    assert route_timeouts(envoy)["ingress_traffic"] == ["1s"]
    assert config["request_timeout"] == "40s"


@pytest.mark.parametrize("value", ["0s", "-1s", "1.5s", "1m", "01s", 120, None])
def test_request_timeout_schema_rejects_invalid_values(tmp_path, value):
    path = tmp_path / "invalid.yaml"
    path.write_text(yaml.safe_dump({"version": "v0.4.0", "request_timeout": value}))
    with pytest.raises(ValidationError):
        validate_prompt_config(path, CONFIG_DIR / "plano_config_schema.yaml")


def test_request_timeout_rejects_proxy_wasm_millisecond_overflow(tmp_path, monkeypatch):
    with pytest.raises(ValueError, match="4294967s"):
        render_config(tmp_path, monkeypatch, request_timeout="4294968s")
