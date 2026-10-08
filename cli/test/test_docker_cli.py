import subprocess
from unittest import mock

from planoai.docker_cli import _get_host_ip


def test_get_host_ip_returns_bridge_gateway():
    """When docker network inspect succeeds, the bridge gateway IP is returned."""
    fake_result = mock.Mock()
    fake_result.returncode = 0
    fake_result.stdout = "172.17.0.1\n"

    with mock.patch("subprocess.run", return_value=fake_result) as mock_run:
        ip = _get_host_ip()

    assert ip == "172.17.0.1"
    mock_run.assert_called_once()
    args = mock_run.call_args[0][0]
    assert "docker" in args
    assert "network" in args
    assert "inspect" in args
    assert "bridge" in args


def test_get_host_ip_falls_back_on_failure():
    """When docker network inspect fails, 'host-gateway' is returned as a fallback."""
    fake_result = mock.Mock()
    fake_result.returncode = 1
    fake_result.stdout = ""

    with mock.patch("subprocess.run", return_value=fake_result):
        ip = _get_host_ip()

    assert ip == "host-gateway"


def test_get_host_ip_falls_back_on_empty_output():
    """When docker network inspect returns empty output, 'host-gateway' is returned."""
    fake_result = mock.Mock()
    fake_result.returncode = 0
    fake_result.stdout = "   "

    with mock.patch("subprocess.run", return_value=fake_result):
        ip = _get_host_ip()

    assert ip == "host-gateway"
