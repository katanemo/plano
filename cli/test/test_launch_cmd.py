"""Tests for the `planoai launch claude-desktop` click command.

Focused on the wiring between the CLI flags and the underlying
`claude_desktop` module / `up` invocation. The actual JSON-rewriting and key
validation are covered in `test_claude_desktop.py`.
"""

from __future__ import annotations

from click.testing import CliRunner

from planoai import claude_desktop as cd
from planoai import launch_cmd as lc


def _stub_cd(monkeypatch):
    """Replace ``claude_desktop`` side-effects with no-ops + call recorders."""
    calls: dict[str, list] = {
        "configure": [],
        "restore": [],
        "launch_or_restart": [],
    }
    monkeypatch.setattr(cd, "supported", lambda: None)
    monkeypatch.setattr(
        cd,
        "configure",
        lambda base_url, **_kw: calls["configure"].append(base_url),
    )
    monkeypatch.setattr(cd, "restore", lambda: calls["restore"].append(True))
    monkeypatch.setattr(
        cd,
        "launch_or_restart",
        lambda prompt, yes: calls["launch_or_restart"].append((prompt, yes)),
    )
    return calls


def test_config_path_starts_plano_when_not_running(tmp_path, monkeypatch):
    config = tmp_path / "plano_config.yaml"
    config.write_text(
        "version: v0.4.0\n"
        "listeners:\n"
        "  - name: llm\n"
        "    type: model\n"
        "    port: 12345\n"
        "    address: 0.0.0.0\n"
        "model_providers: []\n"
    )

    cd_calls = _stub_cd(monkeypatch)
    monkeypatch.setattr(lc, "_is_plano_running", lambda: False)

    up_calls = []

    def fake_up(
        file,
        path,
        foreground,
        with_tracing,
        tracing_port,
        docker,
        verbose,
        listener_port,
    ):
        up_calls.append(
            {
                "file": file,
                "foreground": foreground,
                "docker": docker,
                "listener_port": listener_port,
            }
        )

    from planoai.main import up as up_cmd

    monkeypatch.setattr(up_cmd, "callback", fake_up)

    runner = CliRunner()
    result = runner.invoke(
        lc.launch,
        ["claude-desktop", "--config", str(config), "--yes"],
    )

    assert result.exit_code == 0, result.output
    assert len(up_calls) == 1
    assert up_calls[0]["file"] == str(config)
    assert up_calls[0]["foreground"] is False
    assert cd_calls["configure"] == ["http://localhost:12345"]
    # --yes implies we restart Claude Desktop after configuring.
    assert cd_calls["launch_or_restart"]
    assert cd_calls["launch_or_restart"][0][1] is True


def test_config_path_skips_up_when_plano_already_running(tmp_path, monkeypatch):
    config = tmp_path / "plano_config.yaml"
    config.write_text(
        "version: v0.4.0\n"
        "listeners:\n"
        "  - name: llm\n"
        "    type: model\n"
        "    port: 12500\n"
        "model_providers: []\n"
    )

    cd_calls = _stub_cd(monkeypatch)
    monkeypatch.setattr(lc, "_is_plano_running", lambda: True)

    sentinel = []

    def boom(*args, **kwargs):
        sentinel.append("called")

    from planoai.main import up as up_cmd

    monkeypatch.setattr(up_cmd, "callback", boom)

    runner = CliRunner()
    result = runner.invoke(
        lc.launch,
        ["claude-desktop", "--config", str(config), "--no-launch"],
    )

    assert result.exit_code == 0, result.output
    assert sentinel == [], "should not invoke up.callback when Plano is already running"
    assert cd_calls["configure"] == ["http://localhost:12500"]
    # --no-launch skips the restart step.
    assert cd_calls["launch_or_restart"] == []


def test_config_path_must_exist(tmp_path, monkeypatch):
    cd_calls = _stub_cd(monkeypatch)
    monkeypatch.setattr(lc, "_is_plano_running", lambda: False)

    runner = CliRunner()
    result = runner.invoke(
        lc.launch,
        ["claude-desktop", "--config", str(tmp_path / "nope.yaml")],
    )

    assert result.exit_code != 0
    assert "not found" in result.output.lower()
    assert cd_calls["configure"] == []


def test_no_launch_skips_open(monkeypatch):
    cd_calls = _stub_cd(monkeypatch)
    monkeypatch.setattr(lc, "_is_plano_running", lambda: True)

    runner = CliRunner()
    result = runner.invoke(
        lc.launch,
        ["claude-desktop", "--no-launch", "--base-url", "http://localhost:9999"],
    )

    assert result.exit_code == 0, result.output
    assert cd_calls["configure"] == ["http://localhost:9999"]
    assert cd_calls["launch_or_restart"] == []


def test_restore_ignores_config_path(tmp_path, monkeypatch):
    config = tmp_path / "plano_config.yaml"
    config.write_text("version: v0.4.0\nmodel_providers: []\n")

    cd_calls = _stub_cd(monkeypatch)
    monkeypatch.setattr(lc, "_is_plano_running", lambda: True)

    runner = CliRunner()
    result = runner.invoke(
        lc.launch,
        ["claude-desktop", "--restore", "--config", str(config), "--yes"],
    )

    assert result.exit_code == 0, result.output
    assert cd_calls["restore"] == [True]
    assert cd_calls["configure"] == []
    assert "ignored" in result.output.lower()


def test_base_url_overrides_config_file(tmp_path, monkeypatch):
    config = tmp_path / "plano_config.yaml"
    config.write_text(
        "version: v0.4.0\n"
        "listeners:\n"
        "  - name: llm\n"
        "    type: model\n"
        "    port: 12345\n"
        "model_providers: []\n"
    )

    cd_calls = _stub_cd(monkeypatch)
    monkeypatch.setattr(lc, "_is_plano_running", lambda: True)

    runner = CliRunner()
    result = runner.invoke(
        lc.launch,
        [
            "claude-desktop",
            "--config",
            str(config),
            "--base-url",
            "http://10.0.0.5:8080",
            "--no-launch",
        ],
    )

    assert result.exit_code == 0, result.output
    assert cd_calls["configure"] == ["http://10.0.0.5:8080"]


def test_unsupported_platform_errors(monkeypatch):
    monkeypatch.setattr(
        cd,
        "supported",
        lambda: "Claude Desktop launch is only supported on macOS and Windows",
    )

    runner = CliRunner()
    result = runner.invoke(lc.launch, ["claude-desktop"])

    assert result.exit_code != 0
    assert "macOS" in result.output


def test_help_lists_new_flags(monkeypatch):
    runner = CliRunner()
    result = runner.invoke(lc.launch, ["claude-desktop", "--help"])

    assert result.exit_code == 0, result.output
    assert "--config" in result.output
    assert "--no-launch" in result.output
    assert "--restore" in result.output
