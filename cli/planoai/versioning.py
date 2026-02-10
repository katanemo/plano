import importlib.metadata
import re

PYPI_PACKAGE_NAME = "planoai"
PYPI_URL = f"https://pypi.org/pypi/{PYPI_PACKAGE_NAME}/json"


def get_version() -> str:
    try:
        # First try package metadata (installed package).
        return importlib.metadata.version(PYPI_PACKAGE_NAME)
    except importlib.metadata.PackageNotFoundError:
        # Fallback to local development version.
        try:
            from planoai import __version__

            return __version__
        except ImportError:
            return "version not found"


def get_latest_version(timeout: float = 2.0) -> str | None:
    """Fetch the latest version from PyPI."""
    import requests

    try:
        response = requests.get(PYPI_URL, timeout=timeout)
        if response.status_code == 200:
            data = response.json()
            return data.get("info", {}).get("version")
    except (requests.RequestException, ValueError):
        # Network error or invalid JSON - fail silently.
        return None
    return None


def parse_version(version_str: str) -> tuple[int, ...]:
    """Parse version string into a comparable tuple."""
    clean_version = re.split(r"[a-zA-Z]", version_str)[0]
    parts = clean_version.split(".")
    return tuple(int(p) for p in parts if p.isdigit())


def check_version_status(
    current: str, latest: str | None
) -> dict[str, str | bool | None]:
    """Compare current version with latest and return status metadata."""
    if latest is None:
        return {
            "is_outdated": False,
            "current": current,
            "latest": None,
            "message": None,
        }

    try:
        is_outdated = parse_version(current) < parse_version(latest)
        return {
            "is_outdated": is_outdated,
            "current": current,
            "latest": latest,
            "message": f"Update available: {latest}" if is_outdated else None,
        }
    except (ValueError, TypeError):
        return {
            "is_outdated": False,
            "current": current,
            "latest": latest,
            "message": None,
        }
