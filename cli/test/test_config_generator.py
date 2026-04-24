import json
import pytest
import yaml
from unittest import mock
from planoai.config_generator import (
    validate_and_render_schema,
    migrate_inline_routing_preferences,
)


@pytest.fixture(autouse=True)
def cleanup_env(monkeypatch):
    # Clean up environment variables and mocks after each test
    yield
    monkeypatch.undo()


def test_validate_and_render_happy_path(monkeypatch):
    monkeypatch.setenv("PLANO_CONFIG_FILE", "fake_plano_config.yaml")
    monkeypatch.setenv("PLANO_CONFIG_SCHEMA_FILE", "fake_plano_config_schema.yaml")
    monkeypatch.setenv("ENVOY_CONFIG_TEMPLATE_FILE", "./envoy.template.yaml")
    monkeypatch.setenv("PLANO_CONFIG_FILE_RENDERED", "fake_plano_config_rendered.yaml")
    monkeypatch.setenv("ENVOY_CONFIG_FILE_RENDERED", "fake_envoy.yaml")
    monkeypatch.setenv("TEMPLATE_ROOT", "../")

    plano_config = """
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
        description: understand and explain existing code snippets, functions, or libraries

  - model: openai/gpt-4.1
    access_key: $OPENAI_API_KEY
    routing_preferences:
      - name: code generation
        description: generating new code snippets, functions, or boilerplate based on user prompts or requirements

tracing:
  random_sampling: 100
"""
    plano_config_schema = ""
    with open("../config/plano_config_schema.yaml", "r") as file:
        plano_config_schema = file.read()

    m_open = mock.mock_open()
    # Provide enough file handles for all open() calls in validate_and_render_schema
    m_open.side_effect = [
        # Removed empty read - was causing validation failures
        mock.mock_open(read_data=plano_config).return_value,  # PLANO_CONFIG_FILE
        mock.mock_open(
            read_data=plano_config_schema
        ).return_value,  # PLANO_CONFIG_SCHEMA_FILE
        mock.mock_open(read_data=plano_config).return_value,  # PLANO_CONFIG_FILE
        mock.mock_open(
            read_data=plano_config_schema
        ).return_value,  # PLANO_CONFIG_SCHEMA_FILE
        mock.mock_open().return_value,  # ENVOY_CONFIG_FILE_RENDERED (write)
        mock.mock_open().return_value,  # PLANO_CONFIG_FILE_RENDERED (write)
    ]
    with mock.patch("builtins.open", m_open):
        with mock.patch("planoai.config_generator.Environment"):
            validate_and_render_schema()


def test_validate_and_render_happy_path_agent_config(monkeypatch):
    monkeypatch.setenv("PLANO_CONFIG_FILE", "fake_plano_config.yaml")
    monkeypatch.setenv("PLANO_CONFIG_SCHEMA_FILE", "fake_plano_config_schema.yaml")
    monkeypatch.setenv("ENVOY_CONFIG_TEMPLATE_FILE", "./envoy.template.yaml")
    monkeypatch.setenv("PLANO_CONFIG_FILE_RENDERED", "fake_plano_config_rendered.yaml")
    monkeypatch.setenv("ENVOY_CONFIG_FILE_RENDERED", "fake_envoy.yaml")
    monkeypatch.setenv("TEMPLATE_ROOT", "../")

    plano_config = """
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
        input_filters:
          - query_rewriter
          - context_builder
          - response_generator
      - id: research_agent
        description: agent to research and gather information from various sources.
        input_filters:
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
    plano_config_schema = ""
    with open("../config/plano_config_schema.yaml", "r") as file:
        plano_config_schema = file.read()

    m_open = mock.mock_open()
    # Provide enough file handles for all open() calls in validate_and_render_schema
    m_open.side_effect = [
        # Removed empty read - was causing validation failures
        mock.mock_open(read_data=plano_config).return_value,  # PLANO_CONFIG_FILE
        mock.mock_open(
            read_data=plano_config_schema
        ).return_value,  # PLANO_CONFIG_SCHEMA_FILE
        mock.mock_open(read_data=plano_config).return_value,  # PLANO_CONFIG_FILE
        mock.mock_open(
            read_data=plano_config_schema
        ).return_value,  # PLANO_CONFIG_SCHEMA_FILE
        mock.mock_open().return_value,  # ENVOY_CONFIG_FILE_RENDERED (write)
        mock.mock_open().return_value,  # PLANO_CONFIG_FILE_RENDERED (write)
    ]
    with mock.patch("builtins.open", m_open):
        with mock.patch("planoai.config_generator.Environment"):
            validate_and_render_schema()


plano_config_test_cases = [
    {
        "id": "duplicate_provider_name",
        "expected_error": "Duplicate model_provider name",
        "plano_config": """
version: v0.1.0

listeners:
  egress_traffic:
    address: 0.0.0.0
    port: 12000
    message_format: openai
    timeout: 30s

llm_providers:

  - name: test1
    model: openai/gpt-4o
    access_key: $OPENAI_API_KEY

  - name: test1
    model: openai/gpt-4o
    access_key: $OPENAI_API_KEY

""",
    },
    {
        "id": "provider_interface_with_model_id",
        "expected_error": "Please provide provider interface as part of model name",
        "plano_config": """
version: v0.1.0

listeners:
  egress_traffic:
    address: 0.0.0.0
    port: 12000
    message_format: openai
    timeout: 30s

llm_providers:

  - model: openai/gpt-4o
    access_key: $OPENAI_API_KEY
    provider_interface: openai

""",
    },
    {
        "id": "duplicate_model_id",
        "expected_error": "Duplicate model_id",
        "plano_config": """
version: v0.1.0

listeners:
  egress_traffic:
    address: 0.0.0.0
    port: 12000
    message_format: openai
    timeout: 30s

llm_providers:

  - model: openai/gpt-4o
    access_key: $OPENAI_API_KEY

  - model: mistral/gpt-4o

""",
    },
    {
        "id": "custom_provider_base_url",
        "expected_error": "Must provide base_url and provider_interface",
        "plano_config": """
version: v0.1.0

listeners:
  egress_traffic:
    address: 0.0.0.0
    port: 12000
    message_format: openai
    timeout: 30s

llm_providers:
  - model: custom/gpt-4o

""",
    },
    {
        "id": "base_url_with_path_prefix",
        "expected_error": None,
        "plano_config": """
version: v0.1.0

listeners:
  egress_traffic:
    address: 0.0.0.0
    port: 12000
    message_format: openai
    timeout: 30s

llm_providers:

  - model: custom/gpt-4o
    base_url: "http://custom.com/api/v2"
    provider_interface: openai

""",
    },
    {
        "id": "vercel_is_supported_provider",
        "expected_error": None,
        "plano_config": """
version: v0.4.0

listeners:
  - name: llm
    type: model
    port: 12000

model_providers:
  - model: vercel/*
    base_url: https://ai-gateway.vercel.sh/v1
    passthrough_auth: true

""",
    },
    {
        "id": "openrouter_is_supported_provider",
        "expected_error": None,
        "plano_config": """
version: v0.4.0

listeners:
  - name: llm
    type: model
    port: 12000

model_providers:
  - model: openrouter/*
    base_url: https://openrouter.ai/api/v1
    passthrough_auth: true

""",
    },
    {
        "id": "duplicate_routeing_preference_name",
        "expected_error": "Duplicate routing preference name",
        "plano_config": """
version: v0.4.0

listeners:
  - name: llm
    type: model
    port: 12000

model_providers:
  - model: openai/gpt-4o-mini
    access_key: $OPENAI_API_KEY
    default: true

  - model: openai/gpt-4o
    access_key: $OPENAI_API_KEY

routing_preferences:
  - name: code understanding
    description: understand and explain existing code snippets, functions, or libraries
    models:
      - openai/gpt-4o
  - name: code understanding
    description: generating new code snippets, functions, or boilerplate based on user prompts or requirements
    models:
      - openai/gpt-4o-mini

tracing:
  random_sampling: 100

""",
    },
]


@pytest.mark.parametrize(
    "plano_config_test_case",
    plano_config_test_cases,
    ids=[case["id"] for case in plano_config_test_cases],
)
def test_validate_and_render_schema_tests(monkeypatch, plano_config_test_case):
    monkeypatch.setenv("PLANO_CONFIG_FILE", "fake_plano_config.yaml")
    monkeypatch.setenv("PLANO_CONFIG_SCHEMA_FILE", "fake_plano_config_schema.yaml")
    monkeypatch.setenv("ENVOY_CONFIG_TEMPLATE_FILE", "./envoy.template.yaml")
    monkeypatch.setenv("PLANO_CONFIG_FILE_RENDERED", "fake_plano_config_rendered.yaml")
    monkeypatch.setenv("ENVOY_CONFIG_FILE_RENDERED", "fake_envoy.yaml")
    monkeypatch.setenv("TEMPLATE_ROOT", "../")

    plano_config = plano_config_test_case["plano_config"]
    expected_error = plano_config_test_case.get("expected_error")

    plano_config_schema = ""
    with open("../config/plano_config_schema.yaml", "r") as file:
        plano_config_schema = file.read()

    m_open = mock.mock_open()
    # Provide enough file handles for all open() calls in validate_and_render_schema
    m_open.side_effect = [
        mock.mock_open(
            read_data=plano_config
        ).return_value,  # validate_prompt_config: PLANO_CONFIG_FILE
        mock.mock_open(
            read_data=plano_config_schema
        ).return_value,  # validate_prompt_config: PLANO_CONFIG_SCHEMA_FILE
        mock.mock_open(
            read_data=plano_config
        ).return_value,  # validate_and_render_schema: PLANO_CONFIG_FILE
        mock.mock_open(
            read_data=plano_config_schema
        ).return_value,  # validate_and_render_schema: PLANO_CONFIG_SCHEMA_FILE
        mock.mock_open().return_value,  # ENVOY_CONFIG_FILE_RENDERED (write)
        mock.mock_open().return_value,  # PLANO_CONFIG_FILE_RENDERED (write)
    ]
    with mock.patch("builtins.open", m_open):
        with mock.patch("planoai.config_generator.Environment"):
            if expected_error:
                # Test expects an error
                with pytest.raises(Exception) as excinfo:
                    validate_and_render_schema()
                assert expected_error in str(excinfo.value)
            else:
                # Test expects success - no exception should be raised
                validate_and_render_schema()


def test_convert_legacy_llm_providers():
    from planoai.utils import convert_legacy_listeners

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
        {
            "model": "openai/gpt-4o",
            "access_key": "test_key",
        }
    ]

    updated_providers, llm_gateway, prompt_gateway = convert_legacy_listeners(
        listeners, llm_providers
    )
    assert isinstance(updated_providers, list)
    assert llm_gateway is not None
    assert prompt_gateway is not None
    print(json.dumps(updated_providers))
    assert updated_providers == [
        {
            "name": "egress_traffic",
            "type": "model",
            "port": 12000,
            "address": "0.0.0.0",
            "timeout": "30s",
            "model_providers": [{"model": "openai/gpt-4o", "access_key": "test_key"}],
        },
        {
            "name": "ingress_traffic",
            "type": "prompt",
            "port": 10000,
            "address": "0.0.0.0",
            "timeout": "30s",
        },
    ]

    assert llm_gateway == {
        "address": "0.0.0.0",
        "model_providers": [
            {
                "access_key": "test_key",
                "model": "openai/gpt-4o",
            },
        ],
        "name": "egress_traffic",
        "type": "model",
        "port": 12000,
        "timeout": "30s",
    }

    assert prompt_gateway == {
        "address": "0.0.0.0",
        "name": "ingress_traffic",
        "port": 10000,
        "timeout": "30s",
        "type": "prompt",
    }


def test_convert_legacy_llm_providers_no_prompt_gateway():
    from planoai.utils import convert_legacy_listeners

    listeners = {
        "egress_traffic": {
            "address": "0.0.0.0",
            "port": 12000,
            "timeout": "30s",
        }
    }
    llm_providers = [
        {
            "model": "openai/gpt-4o",
            "access_key": "test_key",
        }
    ]

    updated_providers, llm_gateway, prompt_gateway = convert_legacy_listeners(
        listeners, llm_providers
    )
    assert isinstance(updated_providers, list)
    assert llm_gateway is not None
    assert prompt_gateway is not None
    assert updated_providers == [
        {
            "address": "0.0.0.0",
            "model_providers": [
                {
                    "access_key": "test_key",
                    "model": "openai/gpt-4o",
                },
            ],
            "name": "egress_traffic",
            "port": 12000,
            "timeout": "30s",
            "type": "model",
        }
    ]
    assert llm_gateway == {
        "address": "0.0.0.0",
        "model_providers": [
            {
                "access_key": "test_key",
                "model": "openai/gpt-4o",
            },
        ],
        "name": "egress_traffic",
        "type": "model",
        "port": 12000,
        "timeout": "30s",
    }


def test_inline_routing_preferences_migrated_to_top_level():
    plano_config = """
version: v0.3.0

listeners:
  - type: model
    name: model_listener
    port: 12000

model_providers:
  - model: openai/gpt-4o-mini
    access_key: $OPENAI_API_KEY
    default: true

  - model: openai/gpt-4o
    access_key: $OPENAI_API_KEY
    routing_preferences:
      - name: code understanding
        description: understand and explain existing code snippets, functions, or libraries

  - model: anthropic/claude-sonnet-4-20250514
    access_key: $ANTHROPIC_API_KEY
    routing_preferences:
      - name: code generation
        description: generating new code snippets, functions, or boilerplate based on user prompts or requirements
"""
    config_yaml = yaml.safe_load(plano_config)
    migrate_inline_routing_preferences(config_yaml)

    assert config_yaml["version"] == "v0.4.0"
    for provider in config_yaml["model_providers"]:
        assert "routing_preferences" not in provider

    top_level = config_yaml["routing_preferences"]
    by_name = {entry["name"]: entry for entry in top_level}
    assert set(by_name) == {"code understanding", "code generation"}
    assert by_name["code understanding"]["models"] == ["openai/gpt-4o"]
    assert by_name["code generation"]["models"] == [
        "anthropic/claude-sonnet-4-20250514"
    ]
    assert (
        by_name["code understanding"]["description"]
        == "understand and explain existing code snippets, functions, or libraries"
    )


def test_inline_same_name_across_providers_merges_models():
    plano_config = """
version: v0.3.0

listeners:
  - type: model
    name: model_listener
    port: 12000

model_providers:
  - model: openai/gpt-4o
    access_key: $OPENAI_API_KEY
    routing_preferences:
      - name: code generation
        description: generating new code snippets, functions, or boilerplate based on user prompts or requirements

  - model: anthropic/claude-sonnet-4-20250514
    access_key: $ANTHROPIC_API_KEY
    routing_preferences:
      - name: code generation
        description: generating new code snippets, functions, or boilerplate based on user prompts or requirements
"""
    config_yaml = yaml.safe_load(plano_config)
    migrate_inline_routing_preferences(config_yaml)

    top_level = config_yaml["routing_preferences"]
    assert len(top_level) == 1
    entry = top_level[0]
    assert entry["name"] == "code generation"
    assert entry["models"] == [
        "openai/gpt-4o",
        "anthropic/claude-sonnet-4-20250514",
    ]
    assert config_yaml["version"] == "v0.4.0"


def test_existing_top_level_routing_preferences_preserved():
    plano_config = """
version: v0.4.0

listeners:
  - type: model
    name: model_listener
    port: 12000

model_providers:
  - model: openai/gpt-4o
    access_key: $OPENAI_API_KEY
  - model: anthropic/claude-sonnet-4-20250514
    access_key: $ANTHROPIC_API_KEY

routing_preferences:
  - name: code generation
    description: generating new code snippets or boilerplate
    models:
      - openai/gpt-4o
      - anthropic/claude-sonnet-4-20250514
"""
    config_yaml = yaml.safe_load(plano_config)
    before = yaml.safe_dump(config_yaml, sort_keys=True)
    migrate_inline_routing_preferences(config_yaml)
    after = yaml.safe_dump(config_yaml, sort_keys=True)

    assert before == after


def test_existing_top_level_wins_over_inline_migration():
    plano_config = """
version: v0.3.0

listeners:
  - type: model
    name: model_listener
    port: 12000

model_providers:
  - model: openai/gpt-4o
    access_key: $OPENAI_API_KEY
    routing_preferences:
      - name: code generation
        description: inline description should lose

routing_preferences:
  - name: code generation
    description: user-defined top-level description wins
    models:
      - openai/gpt-4o
"""
    config_yaml = yaml.safe_load(plano_config)
    migrate_inline_routing_preferences(config_yaml)

    top_level = config_yaml["routing_preferences"]
    assert len(top_level) == 1
    entry = top_level[0]
    assert entry["description"] == "user-defined top-level description wins"
    assert entry["models"] == ["openai/gpt-4o"]


def test_wildcard_with_inline_routing_preferences_errors():
    plano_config = """
version: v0.3.0

listeners:
  - type: model
    name: model_listener
    port: 12000

model_providers:
  - model: openrouter/*
    base_url: https://openrouter.ai/api/v1
    passthrough_auth: true
    routing_preferences:
      - name: code generation
        description: generating code
"""
    config_yaml = yaml.safe_load(plano_config)
    with pytest.raises(Exception) as excinfo:
        migrate_inline_routing_preferences(config_yaml)
    assert "wildcard" in str(excinfo.value).lower()


def test_migration_bumps_version_even_without_inline_preferences():
    plano_config = """
version: v0.3.0

listeners:
  - type: model
    name: model_listener
    port: 12000

model_providers:
  - model: openai/gpt-4o
    access_key: $OPENAI_API_KEY
"""
    config_yaml = yaml.safe_load(plano_config)
    migrate_inline_routing_preferences(config_yaml)

    assert "routing_preferences" not in config_yaml
    assert config_yaml["version"] == "v0.4.0"


def test_migration_is_noop_on_v040_config_with_stray_inline_preferences():
    # v0.4.0 configs are assumed to be on the canonical top-level shape.
    # The migration intentionally does not rescue stray inline preferences
    # at v0.4.0+ so that the deprecation boundary is a clean version gate.
    plano_config = """
version: v0.4.0

listeners:
  - type: model
    name: model_listener
    port: 12000

model_providers:
  - model: openai/gpt-4o
    access_key: $OPENAI_API_KEY
    routing_preferences:
      - name: code generation
        description: generating new code
"""
    config_yaml = yaml.safe_load(plano_config)
    migrate_inline_routing_preferences(config_yaml)

    assert config_yaml["version"] == "v0.4.0"
    assert "routing_preferences" not in config_yaml
    assert config_yaml["model_providers"][0]["routing_preferences"] == [
        {"name": "code generation", "description": "generating new code"}
    ]


def test_migration_does_not_downgrade_newer_versions():
    plano_config = """
version: v0.5.0

listeners:
  - type: model
    name: model_listener
    port: 12000

model_providers:
  - model: openai/gpt-4o
    access_key: $OPENAI_API_KEY
"""
    config_yaml = yaml.safe_load(plano_config)
    migrate_inline_routing_preferences(config_yaml)

    assert config_yaml["version"] == "v0.5.0"
