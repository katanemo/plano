import pytest
from unittest import mock
from planoai.versioning import (
    get_version,
    get_latest_version,
    parse_version,
    check_version_status,
    PYPI_URL,
)


class TestParseVersion:
    """Tests for version string parsing."""

    def test_parse_simple_version(self):
        assert parse_version("1.0.0") == (1, 0, 0)
        assert parse_version("0.4.1") == (0, 4, 1)
        assert parse_version("10.20.30") == (10, 20, 30)

    def test_parse_two_part_version(self):
        assert parse_version("1.0") == (1, 0)
        assert parse_version("2.5") == (2, 5)

    def test_parse_version_with_prerelease(self):
        # Pre-release suffixes should be stripped
        assert parse_version("0.4.1a1") == (0, 4, 1)
        assert parse_version("1.0.0beta2") == (1, 0, 0)
        assert parse_version("2.0.0rc1") == (2, 0, 0)


class TestCheckVersionStatus:
    """Tests for version comparison logic."""

    def test_current_equals_latest(self):
        status = check_version_status("0.4.1", "0.4.1")
        assert status["is_outdated"] is False
        assert status["current"] == "0.4.1"
        assert status["latest"] == "0.4.1"
        assert status["message"] is None

    def test_current_is_outdated(self):
        status = check_version_status("0.4.1", "0.5.0")
        assert status["is_outdated"] is True
        assert status["current"] == "0.4.1"
        assert status["latest"] == "0.5.0"
        assert "Update available" in status["message"]
        assert "0.5.0" in status["message"]

    def test_current_is_newer(self):
        # Dev version might be newer than PyPI
        status = check_version_status("0.5.0", "0.4.1")
        assert status["is_outdated"] is False
        assert status["message"] is None

    def test_major_version_outdated(self):
        status = check_version_status("0.4.1", "1.0.0")
        assert status["is_outdated"] is True

    def test_minor_version_outdated(self):
        status = check_version_status("0.4.1", "0.5.0")
        assert status["is_outdated"] is True

    def test_patch_version_outdated(self):
        status = check_version_status("0.4.1", "0.4.2")
        assert status["is_outdated"] is True

    def test_latest_is_none(self):
        # When PyPI check fails
        status = check_version_status("0.4.1", None)
        assert status["is_outdated"] is False
        assert status["latest"] is None
        assert status["message"] is None


class TestGetLatestVersion:
    """Tests for PyPI version fetching."""

    def test_successful_fetch(self):
        mock_response = mock.Mock()
        mock_response.status_code = 200
        mock_response.json.return_value = {"info": {"version": "0.5.0"}}

        with mock.patch("requests.get", return_value=mock_response):
            version = get_latest_version()
            assert version == "0.5.0"

    def test_network_error(self):
        import requests

        with mock.patch(
            "requests.get", side_effect=requests.RequestException("Network error")
        ):
            version = get_latest_version()
            assert version is None

    def test_timeout(self):
        import requests

        with mock.patch("requests.get", side_effect=requests.Timeout("Timeout")):
            version = get_latest_version()
            assert version is None

    def test_invalid_json(self):
        mock_response = mock.Mock()
        mock_response.status_code = 200
        mock_response.json.side_effect = ValueError("Invalid JSON")

        with mock.patch("requests.get", return_value=mock_response):
            version = get_latest_version()
            assert version is None

    def test_404_response(self):
        mock_response = mock.Mock()
        mock_response.status_code = 404

        with mock.patch("requests.get", return_value=mock_response):
            version = get_latest_version()
            assert version is None


class TestVersionCheckIntegration:
    """Integration tests simulating version check scenarios."""

    def test_outdated_version_message(self, capsys):
        """Simulate an outdated version scenario."""
        from rich.console import Console

        console = Console(force_terminal=True)
        current_version = "0.4.1"

        # Mock PyPI returning a newer version
        mock_response = mock.Mock()
        mock_response.status_code = 200
        mock_response.json.return_value = {"info": {"version": "0.5.0"}}

        with mock.patch("requests.get", return_value=mock_response):
            latest = get_latest_version()
            status = check_version_status(current_version, latest)

        assert status["is_outdated"] is True
        assert status["latest"] == "0.5.0"

    def test_up_to_date_version(self):
        """Simulate an up-to-date version scenario."""
        current_version = "0.4.1"

        mock_response = mock.Mock()
        mock_response.status_code = 200
        mock_response.json.return_value = {"info": {"version": "0.4.1"}}

        with mock.patch("requests.get", return_value=mock_response):
            latest = get_latest_version()
            status = check_version_status(current_version, latest)

        assert status["is_outdated"] is False

    def test_skip_version_check_env_var(self, monkeypatch):
        """Test that PLANO_SKIP_VERSION_CHECK skips the check."""
        monkeypatch.setenv("PLANO_SKIP_VERSION_CHECK", "1")

        import os

        assert os.environ.get("PLANO_SKIP_VERSION_CHECK") == "1"
