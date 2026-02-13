"""Model provider constants, custom exception, and URL parsing utility."""

import logging
from urllib.parse import urlparse

log = logging.getLogger(__name__)


class ConfigValidationError(Exception):
    """Raised when config validation fails."""

    pass


# --- Provider Constants ---

SUPPORTED_PROVIDERS_WITH_BASE_URL = [
    "azure_openai",
    "ollama",
    "qwen",
    "amazon_bedrock",
    "arch",
]

SUPPORTED_PROVIDERS_WITHOUT_BASE_URL = [
    "deepseek",
    "groq",
    "mistral",
    "openai",
    "gemini",
    "anthropic",
    "together_ai",
    "xai",
    "moonshotai",
    "zhipu",
]

SUPPORTED_PROVIDERS = (
    SUPPORTED_PROVIDERS_WITHOUT_BASE_URL + SUPPORTED_PROVIDERS_WITH_BASE_URL
)

INTERNAL_PROVIDERS = {
    "arch-function": {
        "name": "arch-function",
        "provider_interface": "arch",
        "model": "Arch-Function",
        "internal": True,
    },
    "plano-orchestrator": {
        "name": "plano-orchestrator",
        "provider_interface": "arch",
        "model": "Plano-Orchestrator",
        "internal": True,
    },
}


def parse_url_endpoint(url):
    """Parse a URL into endpoint, port, protocol, and optional path_prefix.

    Replaces the old get_endpoint_and_port() and inline urlparse logic.
    Raises ConfigValidationError for invalid URLs.

    Returns dict with keys: endpoint, port, protocol, path_prefix (optional)
    """
    result = urlparse(url)
    if not result.scheme or result.scheme not in ("http", "https"):
        raise ConfigValidationError(
            f"Invalid URL '{url}': scheme must be http or https"
        )
    if not result.hostname:
        raise ConfigValidationError(f"Invalid URL '{url}': hostname is required")

    port = result.port
    if port is None:
        port = 80 if result.scheme == "http" else 443

    parsed = {
        "endpoint": result.hostname,
        "port": port,
        "protocol": result.scheme,
    }

    if result.path and result.path != "/":
        parsed["path_prefix"] = result.path

    return parsed
