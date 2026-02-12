"""Tests for config validation, processing, and generation.

Tests are organized in layers:
1. Unit tests for config_providers (parse_url_endpoint, constants)
2. Unit tests for config_validator (pure validation functions, no I/O)
3. Integration tests for validate_and_render_schema (file I/O with tmp_path)
4. Legacy listener conversion tests (unchanged)
"""

import json
import os
import pytest
import yaml
from unittest import mock

from planoai.config_providers import (
    ConfigValidationError,
    SUPPORTED_PROVIDERS,
    parse_url_endpoint,
)
from planoai.config_validator import (
    build_clusters,
    migrate_legacy_providers,
    process_model_providers,
    validate_agents,
    validate_listeners,
    validate_model_aliases,
    validate_prompt_targets,
    validate_schema,
    validate_tracing,
    resolve_agent_orchestrator,
)
from planoai.config_generator import validate_and_render_schema
from planoai.utils import convert_legacy_listeners


# ---------------------------------------------------------------------------
# Layer 1: config_providers unit tests
# ---------------------------------------------------------------------------


class TestParseUrlEndpoint:
    def test_https_with_port(self):
        result = parse_url_endpoint("https://example.com:8443")
        assert result == {
            "endpoint": "example.com",
            "port": 8443,
            "protocol": "https",
        }

    def test_http_default_port(self):
        result = parse_url_endpoint("http://example.com")
        assert result == {
            "endpoint": "example.com",
            "port": 80,
            "protocol": "http",
        }

    def test_https_default_port(self):
        result = parse_url_endpoint("https://example.com")
        assert result == {
            "endpoint": "example.com",
            "port": 443,
            "protocol": "https",
        }

    def test_with_path_prefix(self):
        result = parse_url_endpoint("http://example.com/api/v2")
        assert result["path_prefix"] == "/api/v2"
        assert result["endpoint"] == "example.com"
        assert result["port"] == 80

    def test_invalid_scheme(self):
        with pytest.raises(ConfigValidationError, match="scheme must be http or https"):
            parse_url_endpoint("ftp://example.com")

    def test_no_scheme(self):
        with pytest.raises(ConfigValidationError, match="scheme must be http or https"):
            parse_url_endpoint("example.com")

    def test_no_hostname(self):
        with pytest.raises(ConfigValidationError):
            parse_url_endpoint("http://")


# ---------------------------------------------------------------------------
# Layer 2: config_validator unit tests
# ---------------------------------------------------------------------------


class TestMigrateLegacyProviders:
    def test_no_migration_needed(self):
        config = {"model_providers": [{"model": "openai/gpt-4o"}]}
        result = migrate_legacy_providers(config)
        assert "model_providers" in result
        assert "llm_providers" not in result

    def test_migration_from_llm_providers(self):
        config = {"llm_providers": [{"model": "openai/gpt-4o"}]}
        result = migrate_legacy_providers(config)
        assert result["model_providers"] == [{"model": "openai/gpt-4o"}]
        assert "llm_providers" not in result

    def test_both_present_raises(self):
        config = {"llm_providers": [], "model_providers": []}
        with pytest.raises(ConfigValidationError, match="not both"):
            migrate_legacy_providers(config)

    def test_does_not_mutate_input(self):
        config = {"llm_providers": [{"model": "openai/gpt-4o"}]}
        migrate_legacy_providers(config)
        assert "llm_providers" in config  # original unchanged


class TestValidateAgents:
    def test_duplicate_agent_id(self):
        agents = [
            {"id": "a1", "url": "http://localhost:8000"},
            {"id": "a1", "url": "http://localhost:8001"},
        ]
        with pytest.raises(ConfigValidationError, match="Duplicate agent id"):
            validate_agents(agents, [])

    def test_infers_clusters_from_urls(self):
        agents = [{"id": "a1", "url": "http://localhost:8000"}]
        clusters = validate_agents(agents, [])
        assert "a1" in clusters
        assert clusters["a1"]["port"] == 8000
        assert clusters["a1"]["endpoint"] == "localhost"
        assert clusters["a1"]["protocol"] == "http"

    def test_agents_and_filters_combined(self):
        agents = [{"id": "a1", "url": "http://localhost:8000"}]
        filters = [{"id": "f1", "url": "http://localhost:9000"}]
        clusters = validate_agents(agents, filters)
        assert "a1" in clusters
        assert "f1" in clusters

    def test_duplicate_across_agents_and_filters(self):
        agents = [{"id": "shared", "url": "http://localhost:8000"}]
        filters = [{"id": "shared", "url": "http://localhost:9000"}]
        with pytest.raises(ConfigValidationError, match="Duplicate agent id"):
            validate_agents(agents, filters)

    def test_agent_without_url(self):
        agents = [{"id": "a1"}]
        clusters = validate_agents(agents, [])
        assert clusters == {}


class TestBuildClusters:
    def test_merge_agent_and_explicit_endpoints(self):
        agent_inferred = {
            "agent1": {"endpoint": "localhost", "port": 8000, "protocol": "http"}
        }
        endpoints = {
            "explicit1": {"endpoint": "api.example.com", "port": 443, "protocol": "https"}
        }
        result = build_clusters(endpoints, agent_inferred)
        assert "agent1" in result
        assert "explicit1" in result

    def test_infer_port_from_host_colon_port(self):
        clusters = build_clusters(
            {"svc": {"endpoint": "localhost:9090"}}, {}
        )
        assert clusters["svc"]["endpoint"] == "localhost"
        assert clusters["svc"]["port"] == 9090

    def test_default_port_http(self):
        clusters = build_clusters(
            {"svc": {"endpoint": "localhost", "protocol": "http"}}, {}
        )
        assert clusters["svc"]["port"] == 80


class TestValidatePromptTargets:
    def test_valid_targets(self):
        config = {
            "prompt_targets": [
                {"endpoint": {"name": "my_endpoint"}},
            ]
        }
        clusters = {"my_endpoint": {"endpoint": "localhost", "port": 80}}
        validate_prompt_targets(config, clusters)  # should not raise

    def test_unknown_endpoint(self):
        config = {
            "prompt_targets": [
                {"endpoint": {"name": "nonexistent"}},
            ]
        }
        with pytest.raises(ConfigValidationError, match="Unknown endpoint"):
            validate_prompt_targets(config, {})

    def test_target_without_name(self):
        config = {"prompt_targets": [{"endpoint": {}}]}
        validate_prompt_targets(config, {})  # should not raise


class TestValidateTracing:
    def test_valid_http_endpoint(self):
        result = validate_tracing(
            {"random_sampling": 100},
            "http://host.docker.internal:4317",
        )
        assert result["opentracing_grpc_endpoint"] == "http://host.docker.internal:4317"
        assert result["random_sampling"] == 100

    def test_invalid_scheme(self):
        with pytest.raises(ConfigValidationError, match="scheme must be http"):
            validate_tracing(
                {"opentracing_grpc_endpoint": "https://example.com:4317"},
                "http://default:4317",
            )

    def test_invalid_path(self):
        with pytest.raises(ConfigValidationError, match="path must be empty"):
            validate_tracing(
                {"opentracing_grpc_endpoint": "http://example.com:4317/some/path"},
                "http://default:4317",
            )

    def test_empty_tracing_uses_default(self):
        result = validate_tracing({}, "http://default:4317")
        assert result["opentracing_grpc_endpoint"] == "http://default:4317"


class TestProcessModelProviders:
    def _make_listeners(self, model_providers):
        return [{"model_providers": model_providers}]

    def test_happy_path(self):
        listeners = self._make_listeners([
            {"model": "openai/gpt-4o", "access_key": "$KEY", "default": True},
        ])
        providers, llms, keys = process_model_providers(listeners, {})
        # Should have the user provider + internal providers
        names = [p["name"] for p in providers]
        assert "openai/gpt-4o" in names
        assert "arch-function" in names
        assert "plano-orchestrator" in names

    def test_duplicate_provider_name(self):
        listeners = self._make_listeners([
            {"name": "test1", "model": "openai/gpt-4o", "access_key": "$KEY"},
            {"name": "test1", "model": "openai/gpt-4o-mini", "access_key": "$KEY"},
        ])
        with pytest.raises(ConfigValidationError, match="Duplicate model_provider name"):
            process_model_providers(listeners, {})

    def test_provider_interface_with_supported_provider(self):
        listeners = self._make_listeners([
            {
                "model": "openai/gpt-4o",
                "access_key": "$KEY",
                "provider_interface": "openai",
            },
        ])
        with pytest.raises(
            ConfigValidationError,
            match="provide provider interface as part of model name",
        ):
            process_model_providers(listeners, {})

    def test_duplicate_model_id(self):
        listeners = self._make_listeners([
            {"model": "openai/gpt-4o", "access_key": "$KEY"},
            {"model": "mistral/gpt-4o"},
        ])
        with pytest.raises(ConfigValidationError, match="Duplicate model_id"):
            process_model_providers(listeners, {})

    def test_custom_provider_requires_base_url(self):
        listeners = self._make_listeners([
            {"model": "custom/gpt-4o"},
        ])
        with pytest.raises(
            ConfigValidationError, match="Must provide base_url and provider_interface"
        ):
            process_model_providers(listeners, {})

    def test_base_url_with_path_prefix(self):
        listeners = self._make_listeners([
            {
                "model": "custom/gpt-4o",
                "base_url": "http://custom.com/api/v2",
                "provider_interface": "openai",
            },
        ])
        providers, llms, keys = process_model_providers(listeners, {})
        # Find the custom provider
        custom = next(p for p in providers if p.get("cluster_name"))
        assert custom["base_url_path_prefix"] == "/api/v2"
        assert custom["endpoint"] == "custom.com"
        assert custom["port"] == 80

    def test_duplicate_routing_preference_name(self):
        listeners = self._make_listeners([
            {"model": "openai/gpt-4o-mini", "access_key": "$KEY", "default": True},
            {
                "model": "openai/gpt-4o",
                "access_key": "$KEY",
                "routing_preferences": [
                    {"name": "code understanding", "description": "explains code"},
                ],
            },
            {
                "model": "openai/gpt-4.1",
                "access_key": "$KEY",
                "routing_preferences": [
                    {"name": "code understanding", "description": "generates code"},
                ],
            },
        ])
        with pytest.raises(
            ConfigValidationError, match="Duplicate routing preference name"
        ):
            process_model_providers(listeners, {})

    def test_wildcard_cannot_be_default(self):
        listeners = self._make_listeners([
            {"model": "openai/*", "access_key": "$KEY", "default": True},
        ])
        with pytest.raises(ConfigValidationError, match="Default models cannot be wildcards"):
            process_model_providers(listeners, {})

    def test_invalid_model_name_format(self):
        listeners = self._make_listeners([
            {"model": "gpt-4o", "access_key": "$KEY"},
        ])
        with pytest.raises(ConfigValidationError, match="Invalid model name"):
            process_model_providers(listeners, {})

    def test_internal_providers_always_added(self):
        listeners = self._make_listeners([
            {"model": "openai/gpt-4o", "access_key": "$KEY"},
        ])
        providers, _, _ = process_model_providers(listeners, {})
        names = [p["name"] for p in providers]
        assert "arch-function" in names
        assert "plano-orchestrator" in names

    def test_arch_router_added_when_routing_preferences_exist(self):
        listeners = self._make_listeners([
            {
                "model": "openai/gpt-4o",
                "access_key": "$KEY",
                "routing_preferences": [
                    {"name": "coding", "description": "code tasks"},
                ],
            },
        ])
        providers, _, _ = process_model_providers(listeners, {})
        names = [p["name"] for p in providers]
        assert "arch-router" in names

    def test_skips_listeners_without_model_providers(self):
        listeners = [
            {"name": "agent_listener", "type": "agent"},
            {"model_providers": [{"model": "openai/gpt-4o", "access_key": "$KEY"}]},
        ]
        providers, _, _ = process_model_providers(listeners, {})
        names = [p["name"] for p in providers]
        assert "openai/gpt-4o" in names


class TestValidateListeners:
    def test_single_listener_with_providers(self):
        listeners = [
            {"model_providers": [{"model": "openai/gpt-4o"}]},
            {"name": "agent_listener"},
        ]
        validate_listeners(listeners)  # should not raise

    def test_multiple_listeners_with_providers(self):
        listeners = [
            {"model_providers": [{"model": "openai/gpt-4o"}]},
            {"model_providers": [{"model": "anthropic/claude-3"}]},
        ]
        with pytest.raises(ConfigValidationError, match="not both"):
            validate_listeners(listeners)


class TestValidateModelAliases:
    def test_valid_alias(self):
        validate_model_aliases(
            {"fast": {"target": "gpt-4o"}},
            {"gpt-4o", "gpt-4o-mini"},
        )

    def test_invalid_target(self):
        with pytest.raises(ConfigValidationError, match="not defined as a model"):
            validate_model_aliases(
                {"fast": {"target": "nonexistent"}},
                {"gpt-4o"},
            )

    def test_no_debug_artifact_in_error(self):
        """Regression test: old code had 'Model alias 2 -' debug text."""
        with pytest.raises(ConfigValidationError) as exc_info:
            validate_model_aliases({"fast": {"target": "bad"}}, {"gpt-4o"})
        assert "2 -" not in str(exc_info.value)


class TestResolveAgentOrchestrator:
    def test_not_enabled(self):
        result = resolve_agent_orchestrator({}, {})
        assert result is None

    def test_enabled_with_single_endpoint(self):
        config = {"overrides": {"use_agent_orchestrator": True}}
        endpoints = {"my_agent": {"endpoint": "localhost", "port": 8000}}
        result = resolve_agent_orchestrator(config, endpoints)
        assert result == "my_agent"

    def test_enabled_with_no_endpoints(self):
        config = {"overrides": {"use_agent_orchestrator": True}}
        with pytest.raises(ConfigValidationError, match="provide agent orchestrator"):
            resolve_agent_orchestrator(config, {})

    def test_enabled_with_multiple_endpoints(self):
        config = {"overrides": {"use_agent_orchestrator": True}}
        endpoints = {"a": {}, "b": {}}
        with pytest.raises(ConfigValidationError, match="single agent orchestrator"):
            resolve_agent_orchestrator(config, endpoints)


class TestValidateSchema:
    @pytest.fixture
    def schema(self):
        schema_path = os.path.join(
            os.path.dirname(__file__), "..", "..", "config", "arch_config_schema.yaml"
        )
        with open(schema_path) as f:
            return yaml.safe_load(f.read())

    def test_valid_config(self, schema):
        config = {
            "version": "v0.1.0",
            "listeners": {
                "egress_traffic": {"port": 12000},
            },
        }
        validate_schema(config, schema)  # should not raise

    def test_invalid_config(self, schema):
        config = {"invalid_key": "bad"}
        with pytest.raises(ConfigValidationError, match="Schema validation failed"):
            validate_schema(config, schema)


# ---------------------------------------------------------------------------
# Layer 3: Integration tests
# ---------------------------------------------------------------------------


class TestValidateAndRenderSchema:
    """Integration tests that exercise the full pipeline."""

    @pytest.fixture
    def schema_content(self):
        schema_path = os.path.join(
            os.path.dirname(__file__), "..", "..", "config", "arch_config_schema.yaml"
        )
        with open(schema_path) as f:
            return f.read()

    def test_happy_path_legacy_format(self, tmp_path, schema_content, monkeypatch):
        config_content = """\
version: v0.1.0

listeners:
  egress_traffic:
    address: 0.0.0.0
    port: 12000
    message_format: openai
    timeout: 30s

llm_providers:
  - model: openai/gpt-4o-mini
    access_key: $OPENAI_API_KEY
    default: true

  - model: openai/gpt-4o
    access_key: $OPENAI_API_KEY
    routing_preferences:
      - name: code understanding
        description: understand and explain code

  - model: openai/gpt-4.1
    access_key: $OPENAI_API_KEY
    routing_preferences:
      - name: code generation
        description: generate new code

tracing:
  random_sampling: 100
"""
        config_file = tmp_path / "config.yaml"
        config_file.write_text(config_content)
        schema_file = tmp_path / "schema.yaml"
        schema_file.write_text(schema_content)
        envoy_out = tmp_path / "envoy.yaml"
        config_out = tmp_path / "config_rendered.yaml"

        monkeypatch.setenv("ARCH_CONFIG_FILE", str(config_file))
        monkeypatch.setenv("ARCH_CONFIG_SCHEMA_FILE", str(schema_file))
        monkeypatch.setenv("ENVOY_CONFIG_FILE_RENDERED", str(envoy_out))
        monkeypatch.setenv("ARCH_CONFIG_FILE_RENDERED", str(config_out))

        mock_env = mock.patch("planoai.config_generator.Environment")
        with mock_env as MockEnv:
            mock_template = MockEnv.return_value.get_template.return_value
            mock_template.render.return_value = "# rendered envoy config"
            validate_and_render_schema()

        assert config_out.exists()

    def test_happy_path_agent_config(self, tmp_path, schema_content, monkeypatch):
        config_content = """\
version: v0.3.0

agents:
  - id: query_rewriter
    url: http://localhost:10500
  - id: context_builder
    url: http://localhost:10501
  - id: response_generator
    url: http://localhost:10502
  - id: research_agent
    url: http://localhost:10500
  - id: input_guard_rails
    url: http://localhost:10503

listeners:
  - name: tmobile
    type: agent
    router: plano_orchestrator_v1
    agents:
      - id: simple_tmobile_rag_agent
        description: t-mobile virtual assistant for device contracts.
        filter_chain:
          - query_rewriter
          - context_builder
          - response_generator
      - id: research_agent
        description: agent to research and gather information from various sources.
        filter_chain:
          - research_agent
          - response_generator
    port: 8000

  - name: llm_provider
    type: model
    port: 12000

model_providers:
  - access_key: ${OPENAI_API_KEY}
    model: openai/gpt-4o
"""
        config_file = tmp_path / "config.yaml"
        config_file.write_text(config_content)
        schema_file = tmp_path / "schema.yaml"
        schema_file.write_text(schema_content)
        envoy_out = tmp_path / "envoy.yaml"
        config_out = tmp_path / "config_rendered.yaml"

        monkeypatch.setenv("ARCH_CONFIG_FILE", str(config_file))
        monkeypatch.setenv("ARCH_CONFIG_SCHEMA_FILE", str(schema_file))
        monkeypatch.setenv("ENVOY_CONFIG_FILE_RENDERED", str(envoy_out))
        monkeypatch.setenv("ARCH_CONFIG_FILE_RENDERED", str(config_out))

        mock_env = mock.patch("planoai.config_generator.Environment")
        with mock_env as MockEnv:
            mock_template = MockEnv.return_value.get_template.return_value
            mock_template.render.return_value = "# rendered envoy config"
            validate_and_render_schema()

        assert config_out.exists()


# ---------------------------------------------------------------------------
# Layer 4: Legacy listener conversion tests (unchanged behavior)
# ---------------------------------------------------------------------------


class TestConvertLegacyListeners:
    def test_dict_format_with_both_listeners(self):
        listeners = {
            "ingress_traffic": {
                "address": "0.0.0.0",
                "port": 10000,
                "timeout": "30s",
            },
            "egress_traffic": {
                "address": "0.0.0.0",
                "port": 12000,
                "timeout": "30s",
            },
        }
        llm_providers = [
            {"model": "openai/gpt-4o", "access_key": "test_key"},
        ]

        updated, llm_gateway, prompt_gateway = convert_legacy_listeners(
            listeners, llm_providers
        )
        assert isinstance(updated, list)
        assert llm_gateway is not None
        assert prompt_gateway is not None
        assert updated == [
            {
                "name": "egress_traffic",
                "type": "model_listener",
                "port": 12000,
                "address": "0.0.0.0",
                "timeout": "30s",
                "model_providers": [
                    {"model": "openai/gpt-4o", "access_key": "test_key"}
                ],
            },
            {
                "name": "ingress_traffic",
                "type": "prompt_listener",
                "port": 10000,
                "address": "0.0.0.0",
                "timeout": "30s",
            },
        ]

        assert llm_gateway == {
            "address": "0.0.0.0",
            "model_providers": [
                {"access_key": "test_key", "model": "openai/gpt-4o"},
            ],
            "name": "egress_traffic",
            "type": "model_listener",
            "port": 12000,
            "timeout": "30s",
        }

        assert prompt_gateway == {
            "address": "0.0.0.0",
            "name": "ingress_traffic",
            "port": 10000,
            "timeout": "30s",
            "type": "prompt_listener",
        }

    def test_dict_format_no_prompt_gateway(self):
        listeners = {
            "egress_traffic": {
                "address": "0.0.0.0",
                "port": 12000,
                "timeout": "30s",
            }
        }
        llm_providers = [
            {"model": "openai/gpt-4o", "access_key": "test_key"},
        ]

        updated, llm_gateway, prompt_gateway = convert_legacy_listeners(
            listeners, llm_providers
        )
        assert isinstance(updated, list)
        assert llm_gateway is not None
        assert prompt_gateway is not None
        assert updated == [
            {
                "address": "0.0.0.0",
                "model_providers": [
                    {"access_key": "test_key", "model": "openai/gpt-4o"},
                ],
                "name": "egress_traffic",
                "port": 12000,
                "timeout": "30s",
                "type": "model_listener",
            }
        ]
        assert llm_gateway == {
            "address": "0.0.0.0",
            "model_providers": [
                {"access_key": "test_key", "model": "openai/gpt-4o"},
            ],
            "name": "egress_traffic",
            "type": "model_listener",
            "port": 12000,
            "timeout": "30s",
        }
