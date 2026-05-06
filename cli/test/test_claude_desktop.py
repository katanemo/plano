"""Tests for `planoai launch claude-desktop` configuration logic."""

from __future__ import annotations

import json
import os
from pathlib import Path

import pytest

from planoai import claude_desktop as cd

# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------


@pytest.fixture
def fake_home(tmp_path, monkeypatch):
    """Pretend we're on macOS with a fresh home directory.

    Plano's local gateway has no API key concept, so by default we ensure
    ``$PLANO_API_KEY`` is unset; tests that exercise the env-override path
    re-set it explicitly.
    """
    monkeypatch.setattr(cd, "_GOOS", "darwin")
    monkeypatch.setattr(cd, "_user_home", lambda _: str(tmp_path))
    monkeypatch.delenv("PLANO_API_KEY", raising=False)
    return tmp_path


def _normal_config_path(home: Path) -> Path:
    return (
        home
        / "Library"
        / "Application Support"
        / "Claude"
        / "claude_desktop_config.json"
    )


def _third_party_root(home: Path) -> Path:
    return home / "Library" / "Application Support" / "Claude-3p"


def _third_party_config_path(home: Path) -> Path:
    return _third_party_root(home) / "claude_desktop_config.json"


def _meta_path(home: Path) -> Path:
    return _third_party_root(home) / "configLibrary" / "_meta.json"


def _profile_path(home: Path) -> Path:
    return _third_party_root(home) / "configLibrary" / f"{cd.PROFILE_ID}.json"


# ---------------------------------------------------------------------------
# configure() / restore()
# ---------------------------------------------------------------------------


def test_configure_writes_all_four_files_with_default_api_key(fake_home):
    cd.configure("http://localhost:12000")

    normal_cfg = json.loads(_normal_config_path(fake_home).read_text())
    assert normal_cfg["deploymentMode"] == "3p"

    third_cfg = json.loads(_third_party_config_path(fake_home).read_text())
    assert third_cfg["deploymentMode"] == "3p"

    meta = json.loads(_meta_path(fake_home).read_text())
    assert meta["appliedId"] == cd.PROFILE_ID
    assert any(
        isinstance(e, dict) and e.get("id") == cd.PROFILE_ID for e in meta["entries"]
    )

    profile = json.loads(_profile_path(fake_home).read_text())
    assert profile["inferenceProvider"] == "gateway"
    assert profile["inferenceGatewayBaseUrl"] == "http://localhost:12000"
    # No env override and no pre-existing profile -> placeholder is written.
    assert profile["inferenceGatewayApiKey"] == cd.DEFAULT_API_KEY
    assert profile["inferenceGatewayAuthScheme"] == "bearer"
    assert profile["disableDeploymentModeChooser"] is True
    assert "inferenceModels" not in profile


def test_configure_uses_env_override_when_set(fake_home, monkeypatch):
    monkeypatch.setenv("PLANO_API_KEY", "from-env")
    cd.configure("http://localhost:12000")

    profile = json.loads(_profile_path(fake_home).read_text())
    assert profile["inferenceGatewayApiKey"] == "from-env"


def test_configure_preserves_existing_profile_api_key(fake_home):
    profile = _profile_path(fake_home)
    profile.parent.mkdir(parents=True, exist_ok=True)
    profile.write_text(json.dumps({"inferenceGatewayApiKey": "from-profile"}))

    cd.configure("http://localhost:12000")

    written = json.loads(profile.read_text())
    assert written["inferenceGatewayApiKey"] == "from-profile"


def test_configure_does_not_call_network(fake_home, monkeypatch):
    """Plano's local gateway is not validated at configure time. We must not
    attempt any HTTP request — a 503 from the gateway must not block setup.
    """

    def boom(*_args, **_kwargs):
        raise AssertionError("configure() must not perform network calls")

    monkeypatch.setattr("urllib.request.urlopen", boom)
    cd.configure("http://localhost:12000")

    profile = json.loads(_profile_path(fake_home).read_text())
    assert profile["inferenceProvider"] == "gateway"


def test_configure_preserves_existing_unrelated_keys(fake_home):
    normal_path = _normal_config_path(fake_home)
    normal_path.parent.mkdir(parents=True, exist_ok=True)
    normal_path.write_text(
        json.dumps({"someOtherSetting": 123, "deploymentMode": "1p"})
    )

    cd.configure("http://localhost:12000")

    cfg = json.loads(normal_path.read_text())
    assert cfg["someOtherSetting"] == 123
    assert cfg["deploymentMode"] == "3p"


def test_configure_writes_backup_of_existing_files(fake_home):
    normal_path = _normal_config_path(fake_home)
    normal_path.parent.mkdir(parents=True, exist_ok=True)
    normal_path.write_text('{"deploymentMode":"1p"}')

    cd.configure("http://localhost:12000")

    backup = normal_path.with_suffix(normal_path.suffix + ".bak")
    assert backup.exists()
    assert json.loads(backup.read_text())["deploymentMode"] == "1p"


def test_restore_reverts_deployment_mode_and_strips_gateway_keys(fake_home):
    cd.configure("http://localhost:12000")
    cd.restore()

    assert (
        json.loads(_normal_config_path(fake_home).read_text())["deploymentMode"] == "1p"
    )
    third_cfg = json.loads(_third_party_config_path(fake_home).read_text())
    assert third_cfg["deploymentMode"] == "1p"

    meta = json.loads(_meta_path(fake_home).read_text())
    assert meta.get("appliedId") != cd.PROFILE_ID
    assert all(
        not (isinstance(e, dict) and e.get("id") == cd.PROFILE_ID)
        for e in meta.get("entries", [])
    )

    profile = json.loads(_profile_path(fake_home).read_text())
    assert profile["disableDeploymentModeChooser"] is False
    for stripped in (
        "inferenceProvider",
        "inferenceGatewayBaseUrl",
        "inferenceGatewayAuthScheme",
        "inferenceModels",
    ):
        assert stripped not in profile


def test_restore_meta_keeps_unrelated_entries(fake_home):
    meta_path = _meta_path(fake_home)
    meta_path.parent.mkdir(parents=True, exist_ok=True)
    meta_path.write_text(
        json.dumps(
            {
                "appliedId": cd.PROFILE_ID,
                "entries": [
                    {"id": cd.PROFILE_ID, "name": "Plano"},
                    {"id": "00000000-0000-0000-0000-000000000001", "name": "Other"},
                ],
            }
        )
    )

    cd._restore_meta(str(meta_path))

    meta = json.loads(meta_path.read_text())
    assert meta.get("appliedId") in (None, "")
    ids = [e["id"] for e in meta["entries"] if isinstance(e, dict)]
    assert ids == ["00000000-0000-0000-0000-000000000001"]


# ---------------------------------------------------------------------------
# is_configured()
# ---------------------------------------------------------------------------


def test_is_configured_false_on_fresh_home(fake_home):
    assert cd.is_configured() is False


def test_is_configured_true_after_configure(fake_home):
    cd.configure("http://localhost:12000")
    assert cd.is_configured() is True


def test_is_configured_false_when_only_normal_config_set(fake_home):
    cd.configure("http://localhost:12000")

    third_cfg = _third_party_config_path(fake_home)
    data = json.loads(third_cfg.read_text())
    data["deploymentMode"] = "1p"
    third_cfg.write_text(json.dumps(data))

    assert cd.is_configured() is False


# ---------------------------------------------------------------------------
# API key resolution (placeholder by default; env override; profile preserve)
# ---------------------------------------------------------------------------


def test_resolve_api_key_returns_placeholder_when_no_inputs(fake_home):
    assert cd._resolve_api_key([]) == cd.DEFAULT_API_KEY


def test_resolve_api_key_uses_env_when_set(fake_home, monkeypatch):
    monkeypatch.setenv("PLANO_API_KEY", "from-env")
    profile = _profile_path(fake_home)
    profile.parent.mkdir(parents=True, exist_ok=True)
    profile.write_text(json.dumps({"inferenceGatewayApiKey": "from-profile"}))

    # Env wins over profile.
    assert cd._resolve_api_key([str(profile)]) == "from-env"


def test_resolve_api_key_falls_back_to_existing_profile(fake_home):
    profile = _profile_path(fake_home)
    profile.parent.mkdir(parents=True, exist_ok=True)
    profile.write_text(json.dumps({"inferenceGatewayApiKey": "from-profile"}))

    assert cd._resolve_api_key([str(profile)]) == "from-profile"


def test_resolve_api_key_skips_blank_env(fake_home, monkeypatch):
    monkeypatch.setenv("PLANO_API_KEY", "   ")
    assert cd._resolve_api_key([]) == cd.DEFAULT_API_KEY


# ---------------------------------------------------------------------------
# Atomic write
# ---------------------------------------------------------------------------


def test_atomic_write_creates_backup_of_existing_file(tmp_path):
    target = tmp_path / "deep" / "nested" / "file.json"
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text("ORIGINAL")

    cd._atomic_write_with_backup(str(target), b"NEW")

    assert target.read_text() == "NEW"
    assert (tmp_path / "deep" / "nested" / "file.json.bak").read_text() == "ORIGINAL"


def test_atomic_write_skips_backup_when_no_existing_file(tmp_path):
    target = tmp_path / "fresh.json"
    cd._atomic_write_with_backup(str(target), b"DATA")

    assert target.read_text() == "DATA"
    assert not (tmp_path / "fresh.json.bak").exists()


def test_atomic_write_does_not_truncate_on_failure(tmp_path, monkeypatch):
    target = tmp_path / "file.json"
    target.write_text("ORIGINAL")

    real_replace = os.replace

    def boom(_src, _dst):
        raise OSError("disk full")

    monkeypatch.setattr(os, "replace", boom)
    with pytest.raises(OSError):
        cd._atomic_write_with_backup(str(target), b"NEW")
    monkeypatch.setattr(os, "replace", real_replace)

    assert target.read_text() == "ORIGINAL"
    leftover = list(tmp_path.glob(".plano_*.tmp"))
    assert leftover == []


# ---------------------------------------------------------------------------
# Platform support
# ---------------------------------------------------------------------------


def test_supported_returns_error_on_linux(monkeypatch):
    monkeypatch.setattr(cd, "_GOOS", "linux")
    msg = cd.supported()
    assert msg is not None
    assert "macOS" in msg and "Windows" in msg


def test_supported_returns_none_on_darwin(monkeypatch):
    monkeypatch.setattr(cd, "_GOOS", "darwin")
    assert cd.supported() is None


def test_configure_raises_on_unsupported_platform(monkeypatch):
    monkeypatch.setattr(cd, "_GOOS", "linux")
    with pytest.raises(RuntimeError, match="macOS"):
        cd.configure()


def test_restore_raises_on_unsupported_platform(monkeypatch):
    monkeypatch.setattr(cd, "_GOOS", "linux")
    with pytest.raises(RuntimeError, match="macOS"):
        cd.restore()


# ---------------------------------------------------------------------------
# launch_or_restart()
# ---------------------------------------------------------------------------


def test_launch_or_restart_opens_when_not_running(monkeypatch):
    monkeypatch.setattr(cd, "_GOOS", "darwin")
    monkeypatch.setattr(cd, "_is_running", lambda: False)
    opened = []
    monkeypatch.setattr(cd, "_open", lambda: opened.append(True))
    monkeypatch.setattr(
        cd, "_quit", lambda: pytest.fail("should not quit when not running")
    )

    cd.launch_or_restart("prompt", yes=True)
    assert opened == [True]


def test_launch_or_restart_with_yes_quits_then_opens(monkeypatch):
    monkeypatch.setattr(cd, "_GOOS", "darwin")
    running = [True]
    monkeypatch.setattr(cd, "_is_running", lambda: running[0])

    def quit_app():
        running[0] = False

    quit_calls = []
    open_calls = []
    monkeypatch.setattr(
        cd,
        "_quit",
        lambda: (quit_calls.append(True), quit_app()),
    )
    monkeypatch.setattr(cd, "_open", lambda: open_calls.append(True))
    monkeypatch.setattr(cd, "_sleep", lambda _: None)

    cd.launch_or_restart("Restart?", yes=True)
    assert quit_calls == [True]
    assert open_calls == [True]
