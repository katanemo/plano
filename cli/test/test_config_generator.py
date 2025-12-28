import json
import pytest
from unittest import mock
from planoai.config_generator import validate_and_render_schema


@pytest.fixture(autouse=True)
def cleanup_env(monkeypatch):
    # Clean up environment variables and mocks after each test
    yield
    monkeypatch.undo()


def test_validate_and_render_happy_path(monkeypatch):
    monkeypatch.setenv("ARCH_CONFIG_FILE", "fake_arch_config.yaml")
    monkeypatch.setenv("ARCH_CONFIG_SCHEMA_FILE", "fake_arch_config_schema.yaml")
    monkeypatch.setenv("ENVOY_CONFIG_TEMPLATE_FILE", "./envoy.template.yaml")
    monkeypatch.setenv("ARCH_CONFIG_FILE_RENDERED", "fake_arch_config_rendered.yaml")
    monkeypatch.setenv("ENVOY_CONFIG_FILE_RENDERED", "fake_envoy.yaml")
    monkeypatch.setenv("TEMPLATE_ROOT", "../")

    arch_config = """
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
    arch_config_schema = ""
    with open("../config/arch_config_schema.yaml", "r") as file:
        arch_config_schema = file.read()

    m_open = mock.mock_open()
    # Provide enough file handles for all open() calls in validate_and_render_schema
    m_open.side_effect = [
        # Removed empty read - was causing validation failures
        mock.mock_open(read_data=arch_config).return_value,  # ARCH_CONFIG_FILE
        mock.mock_open(
            read_data=arch_config_schema
        ).return_value,  # ARCH_CONFIG_SCHEMA_FILE
        mock.mock_open(read_data=arch_config).return_value,  # ARCH_CONFIG_FILE
        mock.mock_open(
            read_data=arch_config_schema
        ).return_value,  # ARCH_CONFIG_SCHEMA_FILE
        mock.mock_open().return_value,  # ENVOY_CONFIG_FILE_RENDERED (write)
        mock.mock_open().return_value,  # ARCH_CONFIG_FILE_RENDERED (write)
    ]
    with mock.patch("builtins.open", m_open):
        with mock.patch("planoai.config_generator.Environment"):
            validate_and_render_schema()


def test_validate_and_render_happy_path_agent_config(monkeypatch):
    monkeypatch.setenv("ARCH_CONFIG_FILE", "fake_arch_config.yaml")
    monkeypatch.setenv("ARCH_CONFIG_SCHEMA_FILE", "fake_arch_config_schema.yaml")
    monkeypatch.setenv("ENVOY_CONFIG_TEMPLATE_FILE", "./envoy.template.yaml")
    monkeypatch.setenv("ARCH_CONFIG_FILE_RENDERED", "fake_arch_config_rendered.yaml")
    monkeypatch.setenv("ENVOY_CONFIG_FILE_RENDERED", "fake_envoy.yaml")
    monkeypatch.setenv("TEMPLATE_ROOT", "../")

    arch_config = """
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
      - name: simple_tmobile_rag_agent
        description: t-mobile virtual assistant for device contracts.
        filter_chain:
          - query_rewriter
          - context_builder
          - response_generator
      - name: research_agent
        description: agent to research and gather information from various sources.
        filter_chain:
          - research_agent
          - response_generator
    port: 8000

  - name: llm_provider
    type: model
    description: llm provider configuration
    port: 12000
    llm_providers:
      - access_key: ${OPENAI_API_KEY}
        model: openai/gpt-4o
"""
    arch_config_schema = ""
    with open("../config/arch_config_schema.yaml", "r") as file:
        arch_config_schema = file.read()

    m_open = mock.mock_open()
    # Provide enough file handles for all open() calls in validate_and_render_schema
    m_open.side_effect = [
        # Removed empty read - was causing validation failures
        mock.mock_open(read_data=arch_config).return_value,  # ARCH_CONFIG_FILE
        mock.mock_open(
            read_data=arch_config_schema
        ).return_value,  # ARCH_CONFIG_SCHEMA_FILE
        mock.mock_open(read_data=arch_config).return_value,  # ARCH_CONFIG_FILE
        mock.mock_open(
            read_data=arch_config_schema
        ).return_value,  # ARCH_CONFIG_SCHEMA_FILE
        mock.mock_open().return_value,  # ENVOY_CONFIG_FILE_RENDERED (write)
        mock.mock_open().return_value,  # ARCH_CONFIG_FILE_RENDERED (write)
    ]
    with mock.patch("builtins.open", m_open):
        with mock.patch("planoai.config_generator.Environment"):
            validate_and_render_schema()


arch_config_test_cases = [
    {
        "id": "duplicate_provider_name",
        "expected_error": "Duplicate model_provider name",
        "arch_config": """
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
        "arch_config": """
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
        "arch_config": """
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
        "arch_config": """
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
        "arch_config": """
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
        "id": "duplicate_routeing_preference_name",
        "expected_error": "Duplicate routing preference name",
        "arch_config": """
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
      - name: code understanding
        description: generating new code snippets, functions, or boilerplate based on user prompts or requirements

tracing:
  random_sampling: 100

""",
    },
]


@pytest.mark.parametrize(
    "arch_config_test_case",
    arch_config_test_cases,
    ids=[case["id"] for case in arch_config_test_cases],
)
def test_validate_and_render_schema_tests(monkeypatch, arch_config_test_case):
    monkeypatch.setenv("ARCH_CONFIG_FILE", "fake_arch_config.yaml")
    monkeypatch.setenv("ARCH_CONFIG_SCHEMA_FILE", "fake_arch_config_schema.yaml")
    monkeypatch.setenv("ENVOY_CONFIG_TEMPLATE_FILE", "./envoy.template.yaml")
    monkeypatch.setenv("ARCH_CONFIG_FILE_RENDERED", "fake_arch_config_rendered.yaml")
    monkeypatch.setenv("ENVOY_CONFIG_FILE_RENDERED", "fake_envoy.yaml")
    monkeypatch.setenv("TEMPLATE_ROOT", "../")

    arch_config = arch_config_test_case["arch_config"]
    expected_error = arch_config_test_case.get("expected_error")

    arch_config_schema = ""
    with open("../config/arch_config_schema.yaml", "r") as file:
        arch_config_schema = file.read()

    m_open = mock.mock_open()
    # Provide enough file handles for all open() calls in validate_and_render_schema
    m_open.side_effect = [
        mock.mock_open(
            read_data=arch_config
        ).return_value,  # validate_prompt_config: ARCH_CONFIG_FILE
        mock.mock_open(
            read_data=arch_config_schema
        ).return_value,  # validate_prompt_config: ARCH_CONFIG_SCHEMA_FILE
        mock.mock_open(
            read_data=arch_config
        ).return_value,  # validate_and_render_schema: ARCH_CONFIG_FILE
        mock.mock_open(
            read_data=arch_config_schema
        ).return_value,  # validate_and_render_schema: ARCH_CONFIG_SCHEMA_FILE
        mock.mock_open().return_value,  # ENVOY_CONFIG_FILE_RENDERED (write)
        mock.mock_open().return_value,  # ARCH_CONFIG_FILE_RENDERED (write)
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
            "type": "model_listener",
            "port": 12000,
            "address": "0.0.0.0",
            "timeout": "30s",
            "model_providers": [{"model": "openai/gpt-4o", "access_key": "test_key"}],
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
            {
                "access_key": "test_key",
                "model": "openai/gpt-4o",
            },
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
            "type": "model_listener",
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
        "type": "model_listener",
        "port": 12000,
        "timeout": "30s",
    }
