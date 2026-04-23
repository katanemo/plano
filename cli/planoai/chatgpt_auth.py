"""
ChatGPT subscription OAuth device-flow authentication.

Implements the device code flow used by OpenAI Codex CLI to authenticate
with a ChatGPT Plus/Pro subscription. Tokens are stored locally in
~/.plano/chatgpt/auth.json and auto-refreshed when expired.
"""

import base64
import json
import os
import time
from typing import Any, Dict, Optional, Tuple

import requests

from planoai.consts import PLANO_HOME

# OAuth + API constants (derived from openai/codex)
CHATGPT_AUTH_BASE = "https://auth.openai.com"
CHATGPT_DEVICE_CODE_URL = f"{CHATGPT_AUTH_BASE}/api/accounts/deviceauth/usercode"
CHATGPT_DEVICE_TOKEN_URL = f"{CHATGPT_AUTH_BASE}/api/accounts/deviceauth/token"
CHATGPT_OAUTH_TOKEN_URL = f"{CHATGPT_AUTH_BASE}/oauth/token"
CHATGPT_DEVICE_VERIFY_URL = f"{CHATGPT_AUTH_BASE}/codex/device"
CHATGPT_API_BASE = "https://chatgpt.com/backend-api/codex"
CHATGPT_CLIENT_ID = "app_EMoamEEZ73f0CkXaXp7hrann"

# Local storage
CHATGPT_AUTH_DIR = os.path.join(PLANO_HOME, "chatgpt")
CHATGPT_AUTH_FILE = os.path.join(CHATGPT_AUTH_DIR, "auth.json")

# Timeouts
TOKEN_EXPIRY_SKEW_SECONDS = 60
DEVICE_CODE_TIMEOUT_SECONDS = 15 * 60
DEVICE_CODE_POLL_SECONDS = 5


def _ensure_auth_dir():
    os.makedirs(CHATGPT_AUTH_DIR, exist_ok=True)


def load_auth() -> Optional[Dict[str, Any]]:
    """Load auth data from disk."""
    try:
        with open(CHATGPT_AUTH_FILE, "r") as f:
            return json.load(f)
    except (IOError, json.JSONDecodeError):
        return None


def save_auth(data: Dict[str, Any]):
    """Save auth data to disk."""
    _ensure_auth_dir()
    fd = os.open(CHATGPT_AUTH_FILE, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, "w") as f:
        json.dump(data, f, indent=2)


def delete_auth():
    """Remove stored credentials."""
    try:
        os.remove(CHATGPT_AUTH_FILE)
    except FileNotFoundError:
        pass


def _decode_jwt_claims(token: str) -> Dict[str, Any]:
    """Decode JWT payload without verification."""
    try:
        parts = token.split(".")
        if len(parts) < 2:
            return {}
        payload_b64 = parts[1]
        payload_b64 += "=" * (-len(payload_b64) % 4)
        return json.loads(base64.urlsafe_b64decode(payload_b64).decode("utf-8"))
    except Exception:
        return {}


def _get_expires_at(token: str) -> Optional[int]:
    """Extract expiration time from JWT."""
    claims = _decode_jwt_claims(token)
    exp = claims.get("exp")
    return int(exp) if isinstance(exp, (int, float)) else None


def _extract_account_id(token: Optional[str]) -> Optional[str]:
    """Extract ChatGPT account ID from JWT claims."""
    if not token:
        return None
    claims = _decode_jwt_claims(token)
    auth_claims = claims.get("https://api.openai.com/auth")
    if isinstance(auth_claims, dict):
        account_id = auth_claims.get("chatgpt_account_id")
        if isinstance(account_id, str) and account_id:
            return account_id
    return None


def _is_token_expired(auth_data: Dict[str, Any]) -> bool:
    """Check if the access token is expired."""
    expires_at = auth_data.get("expires_at")
    if expires_at is None:
        access_token = auth_data.get("access_token")
        if access_token:
            expires_at = _get_expires_at(access_token)
            if expires_at:
                auth_data["expires_at"] = expires_at
                save_auth(auth_data)
    if expires_at is None:
        return True
    return time.time() >= float(expires_at) - TOKEN_EXPIRY_SKEW_SECONDS


def _refresh_tokens(refresh_token: str) -> Dict[str, str]:
    """Refresh the access token using the refresh token."""
    resp = requests.post(
        CHATGPT_OAUTH_TOKEN_URL,
        json={
            "client_id": CHATGPT_CLIENT_ID,
            "grant_type": "refresh_token",
            "refresh_token": refresh_token,
            "scope": "openid profile email",
        },
    )
    resp.raise_for_status()
    data = resp.json()

    access_token = data.get("access_token")
    id_token = data.get("id_token")
    if not access_token or not id_token:
        raise RuntimeError(f"Refresh response missing fields: {data}")

    return {
        "access_token": access_token,
        "refresh_token": data.get("refresh_token", refresh_token),
        "id_token": id_token,
    }


def _build_auth_record(tokens: Dict[str, str]) -> Dict[str, Any]:
    """Build the auth record to persist."""
    access_token = tokens.get("access_token")
    id_token = tokens.get("id_token")
    expires_at = _get_expires_at(access_token) if access_token else None
    account_id = _extract_account_id(id_token or access_token)
    return {
        "access_token": access_token,
        "refresh_token": tokens.get("refresh_token"),
        "id_token": id_token,
        "expires_at": expires_at,
        "account_id": account_id,
    }


def request_device_code() -> Dict[str, str]:
    """Request a device code from OpenAI's device auth endpoint."""
    resp = requests.post(
        CHATGPT_DEVICE_CODE_URL,
        json={"client_id": CHATGPT_CLIENT_ID},
    )
    resp.raise_for_status()
    data = resp.json()

    device_auth_id = data.get("device_auth_id")
    user_code = data.get("user_code") or data.get("usercode")
    interval = data.get("interval")
    if not device_auth_id or not user_code:
        raise RuntimeError(f"Device code response missing fields: {data}")

    return {
        "device_auth_id": device_auth_id,
        "user_code": user_code,
        "interval": str(interval or "5"),
    }


def poll_for_authorization(device_code: Dict[str, str]) -> Dict[str, str]:
    """Poll until the user completes authorization. Returns code_data."""
    interval = int(device_code.get("interval", "5"))
    start_time = time.time()

    while time.time() - start_time < DEVICE_CODE_TIMEOUT_SECONDS:
        try:
            resp = requests.post(
                CHATGPT_DEVICE_TOKEN_URL,
                json={
                    "device_auth_id": device_code["device_auth_id"],
                    "user_code": device_code["user_code"],
                },
            )
            if resp.status_code == 200:
                data = resp.json()
                if all(
                    key in data
                    for key in ("authorization_code", "code_challenge", "code_verifier")
                ):
                    return data
            if resp.status_code in (403, 404):
                time.sleep(max(interval, DEVICE_CODE_POLL_SECONDS))
                continue
            resp.raise_for_status()
        except requests.HTTPError as exc:
            if exc.response is not None and exc.response.status_code in (403, 404):
                time.sleep(max(interval, DEVICE_CODE_POLL_SECONDS))
                continue
            raise RuntimeError(f"Polling failed: {exc}") from exc

        time.sleep(max(interval, DEVICE_CODE_POLL_SECONDS))

    raise RuntimeError("Timed out waiting for device authorization")


def exchange_code_for_tokens(code_data: Dict[str, str]) -> Dict[str, str]:
    """Exchange the authorization code for access/refresh/id tokens."""
    redirect_uri = f"{CHATGPT_AUTH_BASE}/deviceauth/callback"
    body = (
        "grant_type=authorization_code"
        f"&code={code_data['authorization_code']}"
        f"&redirect_uri={redirect_uri}"
        f"&client_id={CHATGPT_CLIENT_ID}"
        f"&code_verifier={code_data['code_verifier']}"
    )
    resp = requests.post(
        CHATGPT_OAUTH_TOKEN_URL,
        headers={"Content-Type": "application/x-www-form-urlencoded"},
        data=body,
    )
    resp.raise_for_status()
    data = resp.json()

    if not all(key in data for key in ("access_token", "refresh_token", "id_token")):
        raise RuntimeError(f"Token exchange response missing fields: {data}")

    return {
        "access_token": data["access_token"],
        "refresh_token": data["refresh_token"],
        "id_token": data["id_token"],
    }


def login() -> Dict[str, Any]:
    """Run the full device code login flow. Returns the auth record."""
    device_code = request_device_code()
    auth_record = _build_auth_record({})
    auth_record["device_code_requested_at"] = time.time()
    save_auth(auth_record)

    print(
        "\nSign in with your ChatGPT account:\n"
        f"  1) Visit: {CHATGPT_DEVICE_VERIFY_URL}\n"
        f"  2) Enter code: {device_code['user_code']}\n\n"
        "Device codes are a common phishing target. Never share this code.\n",
        flush=True,
    )

    code_data = poll_for_authorization(device_code)
    tokens = exchange_code_for_tokens(code_data)
    auth_record = _build_auth_record(tokens)
    save_auth(auth_record)
    return auth_record


def get_access_token() -> Tuple[str, Optional[str]]:
    """
    Get a valid access token and account ID.
    Refreshes automatically if expired. Raises if no auth data exists.
    Returns (access_token, account_id).
    """
    auth_data = load_auth()
    if not auth_data:
        raise RuntimeError(
            "No ChatGPT credentials found. Run 'planoai chatgpt login' first."
        )

    access_token = auth_data.get("access_token")
    if access_token and not _is_token_expired(auth_data):
        return access_token, auth_data.get("account_id")

    # Try refresh
    refresh_token = auth_data.get("refresh_token")
    if refresh_token:
        tokens = _refresh_tokens(refresh_token)
        auth_record = _build_auth_record(tokens)
        save_auth(auth_record)
        return auth_record["access_token"], auth_record.get("account_id")

    raise RuntimeError(
        "ChatGPT token expired and refresh failed. Run 'planoai chatgpt login' again."
    )
