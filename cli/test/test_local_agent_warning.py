"""Tests for the local-agent provider warning, ack persistence, and the
detection logic that decides whether to fire it."""

from __future__ import annotations

import io
import json

import pytest
from rich.console import Console

from planoai import local_agent_warning as law


def _make_console() -> tuple[Console, io.StringIO]:
    buf = io.StringIO()
    # ``force_terminal=False`` keeps Rich from emitting ANSI escapes,
    # which makes substring assertions readable. ``width`` is generous
    # so the panel border doesn't soft-wrap text mid-keyword.
    console = Console(file=buf, force_terminal=False, color_system=None, width=140)
    return console, buf


# ---------------------------------------------------------------------------
# detection
# ---------------------------------------------------------------------------


def test_detects_claude_cli_via_model_prefix():
    config = {
        "model_providers": [
            {"model": "claude-cli/sonnet"},
            {"model": "openai/gpt-4o"},
        ]
    }
    found = law.detect_local_agent_providers(config)
    assert [p.interface for p in found] == ["claude-cli"]
    assert found[0].model == "claude-cli/sonnet"


def test_detects_claude_cli_via_explicit_provider_interface():
    config = {
        "model_providers": [
            {"name": "local-claude", "provider_interface": "claude-cli", "model": "x"},
        ]
    }
    found = law.detect_local_agent_providers(config)
    assert [p.interface for p in found] == ["claude-cli"]
    assert found[0].name == "local-claude"


def test_detects_claude_cli_via_legacy_provider_field():
    config = {"model_providers": [{"provider": "claude-cli", "model": "x"}]}
    assert [p.interface for p in law.detect_local_agent_providers(config)] == [
        "claude-cli"
    ]


def test_detects_via_legacy_llm_providers_key():
    config = {"llm_providers": [{"model": "claude-cli/opus"}]}
    assert [p.interface for p in law.detect_local_agent_providers(config)] == [
        "claude-cli"
    ]


def test_no_false_positive_for_network_providers():
    config = {
        "model_providers": [
            {"model": "openai/gpt-4o"},
            {"model": "anthropic/claude-3-5-sonnet"},
            {"model": "gemini/gemini-2.5-pro"},
            {"model": "chatgpt/gpt-5"},  # network ChatGPT subscription, not a CLI
            {"model": "vercel/some-model"},
        ]
    }
    assert law.detect_local_agent_providers(config) == []


def test_no_false_positive_for_anthropic_claude_models():
    # ``anthropic/claude-3-5-sonnet`` must not trigger just because the
    # word "claude" appears — the prefix has to be ``claude-cli/``.
    config = {"model_providers": [{"model": "anthropic/claude-3-5-sonnet-20241022"}]}
    assert law.detect_local_agent_providers(config) == []


def test_empty_or_malformed_config_is_safe():
    assert law.detect_local_agent_providers({}) == []
    assert law.detect_local_agent_providers({"model_providers": None}) == []
    assert law.detect_local_agent_providers({"model_providers": "not-a-list"}) == []
    # ``None`` config (e.g. from an empty yaml file) must also be safe.
    assert law.detect_local_agent_providers(None) == []  # type: ignore[arg-type]


def test_multiple_entries_same_interface_collapse_in_warning_set():
    config = {
        "model_providers": [
            {"model": "claude-cli/sonnet", "name": "fast"},
            {"model": "claude-cli/opus", "name": "slow"},
        ]
    }
    found = law.detect_local_agent_providers(config)
    assert len(found) == 2
    assert {p.interface for p in found} == {"claude-cli"}


# ---------------------------------------------------------------------------
# ack file
# ---------------------------------------------------------------------------


def test_load_ack_returns_empty_when_missing(tmp_path):
    ack = tmp_path / "ack.json"
    assert law.load_acknowledged_interfaces(str(ack)) == set()


@pytest.mark.parametrize(
    "contents",
    [
        "{not valid json",
        "[]",  # not a dict
        '{"acknowledged": "claude-cli"}',  # not a list
        '{"acknowledged": [1, 2, 3]}',  # not strings
    ],
)
def test_load_ack_handles_malformed_files(tmp_path, contents):
    ack = tmp_path / "ack.json"
    ack.write_text(contents, encoding="utf-8")
    # Malformed contents must degrade to "no ack" rather than crashing.
    assert law.load_acknowledged_interfaces(str(ack)) == set()


def test_write_ack_creates_state_dir(tmp_path):
    ack = tmp_path / "fresh" / "deeper" / "ack.json"
    merged = law.write_acknowledgement(["claude-cli"], ack_path=str(ack))
    assert merged == {"claude-cli"}
    assert ack.exists()
    payload = json.loads(ack.read_text(encoding="utf-8"))
    assert payload["acknowledged"] == ["claude-cli"]
    assert payload["ack_at"]


def test_write_ack_merges_with_existing(tmp_path):
    ack = tmp_path / "ack.json"
    law.write_acknowledgement(["claude-cli"], ack_path=str(ack))
    merged = law.write_acknowledgement(["future-cli"], ack_path=str(ack))
    assert merged == {"claude-cli", "future-cli"}
    payload = json.loads(ack.read_text(encoding="utf-8"))
    assert payload["acknowledged"] == ["claude-cli", "future-cli"]


# ---------------------------------------------------------------------------
# maybe_warn_local_agent_providers
# ---------------------------------------------------------------------------


def test_no_panel_when_no_local_agent_providers(tmp_path):
    console, buf = _make_console()
    fired = law.maybe_warn_local_agent_providers(
        {"model_providers": [{"model": "openai/gpt-4o"}]},
        console,
        ack_path=str(tmp_path / "ack.json"),
        env={},
    )
    assert fired is False
    assert buf.getvalue() == ""


def test_panel_fires_for_unacked_claude_cli(tmp_path):
    console, buf = _make_console()
    fired = law.maybe_warn_local_agent_providers(
        {"model_providers": [{"model": "claude-cli/sonnet"}]},
        console,
        ack_path=str(tmp_path / "ack.json"),
        env={},
    )
    output = buf.getvalue()
    assert fired is True
    # Stable substrings — never pin exact wording.
    assert "claude-cli" in output
    assert "Local-agent" in output or "local-agent" in output
    assert "Learn more" in output
    assert "--ack-local-agents" in output
    # The dismissal hint must mention the ack file path so the user
    # knows where to ``rm`` it.
    assert "local_agent_ack.json" in output


def test_panel_suppressed_when_ack_covers_interface(tmp_path):
    ack = tmp_path / "ack.json"
    law.write_acknowledgement(["claude-cli"], ack_path=str(ack))

    console, buf = _make_console()
    fired = law.maybe_warn_local_agent_providers(
        {"model_providers": [{"model": "claude-cli/sonnet"}]},
        console,
        ack_path=str(ack),
        env={},
    )
    assert fired is False
    # The dim INFO line still mentions the ack file so the operator
    # knows how to undo, but no panel renders.
    out = buf.getvalue()
    assert "Panel" not in out  # no panel object
    assert "claude-cli" in out


def test_new_unacked_interface_re_triggers(tmp_path, monkeypatch):
    # Simulate a future where two local-agent interfaces exist and the
    # user has only acknowledged one of them.
    monkeypatch.setattr(
        law, "LOCAL_AGENT_PROVIDER_INTERFACES", ("claude-cli", "future-cli")
    )

    ack = tmp_path / "ack.json"
    law.write_acknowledgement(["claude-cli"], ack_path=str(ack))

    console, buf = _make_console()
    fired = law.maybe_warn_local_agent_providers(
        {
            "model_providers": [
                {"model": "claude-cli/sonnet"},
                {"model": "future-cli/whatever"},
            ]
        },
        console,
        ack_path=str(ack),
        env={},
    )
    output = buf.getvalue()
    assert fired is True
    # The panel must list the *unacknowledged* interface only.
    assert "future-cli" in output
    # ...and must NOT re-list the already-acknowledged one as unacked
    # (it can still appear in the suppressed-info line; we check the
    # title which only contains pending interfaces).
    assert "future-cli" in output


def test_ack_flag_writes_file_and_suppresses_panel(tmp_path):
    ack = tmp_path / "ack.json"
    console, buf = _make_console()
    fired = law.maybe_warn_local_agent_providers(
        {"model_providers": [{"model": "claude-cli/sonnet"}]},
        console,
        ack_flag=True,
        ack_path=str(ack),
        env={},
    )
    assert fired is False
    assert ack.exists()
    payload = json.loads(ack.read_text(encoding="utf-8"))
    assert "claude-cli" in payload["acknowledged"]
    out = buf.getvalue()
    assert "Acknowledged" in out
    assert "claude-cli" in out


@pytest.mark.parametrize("env_value", ["1", "true", "TRUE", "yes", "on"])
def test_ack_env_var_truthy_values(tmp_path, env_value):
    ack = tmp_path / "ack.json"
    console, _ = _make_console()
    fired = law.maybe_warn_local_agent_providers(
        {"model_providers": [{"model": "claude-cli/sonnet"}]},
        console,
        ack_path=str(ack),
        env={law.ACK_ENV_VAR: env_value},
    )
    assert fired is False
    assert ack.exists()


@pytest.mark.parametrize("env_value", ["", "0", "false", "no", "off", "maybe"])
def test_ack_env_var_falsy_values_still_warn(tmp_path, env_value):
    ack = tmp_path / "ack.json"
    console, buf = _make_console()
    fired = law.maybe_warn_local_agent_providers(
        {"model_providers": [{"model": "claude-cli/sonnet"}]},
        console,
        ack_path=str(ack),
        env={law.ACK_ENV_VAR: env_value},
    )
    assert fired is True
    assert not ack.exists()
    assert "claude-cli" in buf.getvalue()


def test_malformed_ack_falls_back_to_warning(tmp_path):
    ack = tmp_path / "ack.json"
    ack.write_text("{not json", encoding="utf-8")
    console, buf = _make_console()
    fired = law.maybe_warn_local_agent_providers(
        {"model_providers": [{"model": "claude-cli/sonnet"}]},
        console,
        ack_path=str(ack),
        env={},
    )
    assert fired is True
    assert "claude-cli" in buf.getvalue()


def test_single_panel_when_multiple_local_agent_entries(tmp_path):
    # Two entries with the same interface must produce one panel,
    # not two — the warning fires once per ``planoai up`` invocation.
    console, buf = _make_console()
    fired = law.maybe_warn_local_agent_providers(
        {
            "model_providers": [
                {"model": "claude-cli/sonnet", "name": "fast"},
                {"model": "claude-cli/opus", "name": "slow"},
            ]
        },
        console,
        ack_path=str(tmp_path / "ack.json"),
        env={},
    )
    assert fired is True
    output = buf.getvalue()
    # Both names appear in the listing, but the warning header
    # (``Local-agent provider detected``) appears exactly once.
    assert output.count("Local-agent provider detected") == 1
    assert "fast" in output
    assert "slow" in output
