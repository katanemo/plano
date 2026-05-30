"""Unit tests for the claude-cli env wiring in native_runner.py."""

import os
import textwrap

from planoai.native_runner import (
    CLAUDE_CLI_DEFAULT_LISTEN_ADDR,
    _apply_claude_cli_env,
    _needs_claude_cli_runtime,
)


def _write(path, body):
    path.write_text(textwrap.dedent(body).lstrip())
    return str(path)


def test_needs_claude_cli_runtime_detects_provider(tmp_path):
    rendered = _write(
        tmp_path / "rendered.yaml",
        """
        version: v0.4.0
        listeners: []
        model_providers:
          - name: claude-cli/*
            model: '*'
            provider_interface: claude-cli
            base_url: http://127.0.0.1:14001
        """,
    )
    assert _needs_claude_cli_runtime(rendered) is True


def test_needs_claude_cli_runtime_skips_other_providers(tmp_path):
    rendered = _write(
        tmp_path / "rendered.yaml",
        """
        version: v0.4.0
        model_providers:
          - name: openai/gpt-4o
            model: gpt-4o
            provider_interface: openai
        """,
    )
    assert _needs_claude_cli_runtime(rendered) is False


def test_needs_claude_cli_runtime_handles_missing_file(tmp_path):
    assert _needs_claude_cli_runtime(str(tmp_path / "does-not-exist.yaml")) is False


def test_apply_claude_cli_env_injects_default_addr(tmp_path, monkeypatch):
    rendered = _write(
        tmp_path / "rendered.yaml",
        """
        model_providers:
          - provider_interface: claude-cli
            model: '*'
        """,
    )
    monkeypatch.delenv("CLAUDE_CLI_LISTEN_ADDR", raising=False)
    monkeypatch.delenv("CLAUDE_CLI_BIN", raising=False)
    env = {}
    assert _apply_claude_cli_env(env, rendered) is True
    assert env["CLAUDE_CLI_LISTEN_ADDR"] == CLAUDE_CLI_DEFAULT_LISTEN_ADDR


def test_apply_claude_cli_env_honors_user_override(tmp_path, monkeypatch):
    rendered = _write(
        tmp_path / "rendered.yaml",
        """
        model_providers:
          - provider_interface: claude-cli
            model: '*'
        """,
    )
    monkeypatch.delenv("CLAUDE_CLI_LISTEN_ADDR", raising=False)
    env = {"CLAUDE_CLI_LISTEN_ADDR": "127.0.0.1:25000"}
    assert _apply_claude_cli_env(env, rendered) is True
    assert env["CLAUDE_CLI_LISTEN_ADDR"] == "127.0.0.1:25000"


def test_apply_claude_cli_env_passes_through_user_env(tmp_path, monkeypatch):
    rendered = _write(
        tmp_path / "rendered.yaml",
        """
        model_providers:
          - provider_interface: claude-cli
            model: '*'
        """,
    )
    monkeypatch.delenv("CLAUDE_CLI_LISTEN_ADDR", raising=False)
    monkeypatch.setenv("CLAUDE_CLI_BIN", "/usr/local/bin/claude-test")
    monkeypatch.setenv("CLAUDE_CLI_PERMISSION_MODE", "default")
    env = {}
    assert _apply_claude_cli_env(env, rendered) is True
    assert env["CLAUDE_CLI_BIN"] == "/usr/local/bin/claude-test"
    assert env["CLAUDE_CLI_PERMISSION_MODE"] == "default"


def test_apply_claude_cli_env_noop_for_other_configs(tmp_path):
    rendered = _write(
        tmp_path / "rendered.yaml",
        """
        model_providers:
          - provider_interface: openai
            model: gpt-4o
        """,
    )
    env = {}
    assert _apply_claude_cli_env(env, rendered) is False
    assert "CLAUDE_CLI_LISTEN_ADDR" not in env
