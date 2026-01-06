import pytest
import os
import tempfile
from unittest import mock
from click.testing import CliRunner
from planoai.main import validate, validate_config_file


class TestValidateConfigFile:
    """Tests for the validate_config_file function."""

    def test_valid_config(self, tmp_path):
        """Test validation of a valid config file."""
        config_content = """
version: v0.3.0

listeners:
  - type: model
    name: llm_gateway
    port: 12000

model_providers:
  - model: openai/gpt-4o-mini
    access_key: $OPENAI_API_KEY
    default: true
"""
        config_file = tmp_path / "config.yaml"
        config_file.write_text(config_content)

        result = validate_config_file(str(config_file))

        assert result["valid"] is True
        assert len(result["errors"]) == 0
        assert result["config"] is not None
        assert len(result["summary"]["model_providers"]) == 1
        assert result["summary"]["model_providers"][0]["model"] == "openai/gpt-4o-mini"
        assert result["summary"]["model_providers"][0]["default"] is True
        assert "OPENAI_API_KEY" in result["summary"]["env_vars_required"]

    def test_invalid_yaml_syntax(self, tmp_path):
        """Test validation fails for invalid YAML syntax."""
        config_content = """
version: v0.3.0
listeners:
  - type: model
    name: [invalid yaml
"""
        config_file = tmp_path / "config.yaml"
        config_file.write_text(config_content)

        result = validate_config_file(str(config_file))

        assert result["valid"] is False
        assert any("Invalid YAML" in error for error in result["errors"])

    def test_file_not_found(self):
        """Test validation fails for non-existent file."""
        result = validate_config_file("/nonexistent/path/config.yaml")

        assert result["valid"] is False
        assert any("not found" in error for error in result["errors"])

    def test_multiple_model_providers(self, tmp_path):
        """Test config with multiple model providers."""
        config_content = """
version: v0.3.0

listeners:
  - type: model
    name: llm_gateway
    port: 12000

model_providers:
  - model: openai/gpt-4o-mini
    access_key: $OPENAI_API_KEY
    default: true

  - model: anthropic/claude-3-5-sonnet
    access_key: $ANTHROPIC_API_KEY

  - model: mistral/mistral-large
    access_key: $MISTRAL_API_KEY
"""
        config_file = tmp_path / "config.yaml"
        config_file.write_text(config_content)

        result = validate_config_file(str(config_file))

        assert result["valid"] is True
        assert len(result["summary"]["model_providers"]) == 3
        assert set(result["summary"]["env_vars_required"]) == {
            "OPENAI_API_KEY",
            "ANTHROPIC_API_KEY",
            "MISTRAL_API_KEY",
        }

    def test_legacy_listener_format(self, tmp_path):
        """Test config with legacy listener format."""
        config_content = """
version: v0.1.0

listeners:
  egress_traffic:
    address: 0.0.0.0
    port: 12000
  ingress_traffic:
    address: 0.0.0.0
    port: 10000

llm_providers:
  - model: openai/gpt-4o
    access_key: $OPENAI_API_KEY
"""
        config_file = tmp_path / "config.yaml"
        config_file.write_text(config_content)

        result = validate_config_file(str(config_file))

        assert result["valid"] is True
        assert len(result["summary"]["listeners"]) == 2

        # Check listener ports are correctly extracted
        ports = [l["port"] for l in result["summary"]["listeners"]]
        assert 12000 in ports
        assert 10000 in ports


class TestValidateCommand:
    """Tests for the CLI validate command."""

    def test_validate_command_with_valid_file(self, tmp_path):
        """Test validate command with a valid config file."""
        config_content = """
version: v0.3.0

listeners:
  - type: model
    name: llm_gateway
    port: 12000

model_providers:
  - model: openai/gpt-4o-mini
    access_key: $OPENAI_API_KEY
    default: true
"""
        config_file = tmp_path / "config.yaml"
        config_file.write_text(config_content)

        runner = CliRunner()
        result = runner.invoke(validate, [str(config_file)])

        assert result.exit_code == 0
        assert "valid" in result.output.lower() or "✓" in result.output

    def test_validate_command_with_invalid_file(self, tmp_path):
        """Test validate command fails with invalid config."""
        config_content = """
version: v0.3.0
invalid_yaml: [broken
"""
        config_file = tmp_path / "config.yaml"
        config_file.write_text(config_content)

        runner = CliRunner()
        result = runner.invoke(validate, [str(config_file)])

        assert result.exit_code == 1
        assert "invalid" in result.output.lower() or "✗" in result.output

    def test_validate_command_auto_detect_config(self, tmp_path, monkeypatch):
        """Test validate command auto-detects config.yaml in current directory."""
        config_content = """
version: v0.3.0

listeners:
  - type: model
    name: test
    port: 12000

model_providers:
  - model: openai/gpt-4o
    access_key: $OPENAI_API_KEY
"""
        config_file = tmp_path / "config.yaml"
        config_file.write_text(config_content)

        # Change to the temp directory
        monkeypatch.chdir(tmp_path)

        runner = CliRunner()
        result = runner.invoke(validate, [])

        assert result.exit_code == 0
        assert "valid" in result.output.lower() or "✓" in result.output

    def test_validate_command_quiet_mode(self, tmp_path):
        """Test validate command with --quiet flag."""
        config_content = """
version: v0.3.0

listeners:
  - type: model
    name: llm_gateway
    port: 12000

model_providers:
  - model: openai/gpt-4o-mini
    access_key: $OPENAI_API_KEY
"""
        config_file = tmp_path / "config.yaml"
        config_file.write_text(config_content)

        runner = CliRunner()
        result = runner.invoke(validate, [str(config_file), "--quiet"])

        assert result.exit_code == 0
        # Quiet mode should have minimal output (no tables)
        assert "Model Providers" not in result.output
