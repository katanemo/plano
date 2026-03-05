from unittest import mock

from planoai.core import start_cli_agent


PLANO_CONFIG = """
version: v0.3.0
listeners:
  egress_traffic:
    host: 127.0.0.1
    port: 12000
"""


def test_start_cli_agent_codex_defaults():
    with mock.patch("builtins.open", mock.mock_open(read_data=PLANO_CONFIG)):
        with mock.patch("subprocess.run") as mock_run:
            start_cli_agent("fake_plano_config.yaml", "codex", "{}")

    mock_run.assert_called_once()
    args, kwargs = mock_run.call_args
    assert args[0] == ["codex", "--model", "gpt-5.3-codex"]
    assert kwargs["check"] is True
    assert kwargs["env"]["OPENAI_BASE_URL"] == "http://127.0.0.1:12000/v1"
    assert kwargs["env"]["OPENAI_API_KEY"] == "test"


def test_start_cli_agent_claude_keeps_existing_flow():
    with mock.patch("builtins.open", mock.mock_open(read_data=PLANO_CONFIG)):
        with mock.patch("subprocess.run") as mock_run:
            start_cli_agent(
                "fake_plano_config.yaml",
                "claude",
                '{"NON_INTERACTIVE_MODE": true}',
            )

    mock_run.assert_called_once()
    args, kwargs = mock_run.call_args
    assert args[0] == ["claude"]
    assert kwargs["check"] is True
    assert kwargs["env"]["ANTHROPIC_BASE_URL"] == "http://127.0.0.1:12000"
    assert kwargs["env"]["ANTHROPIC_AUTH_TOKEN"] == "test"
