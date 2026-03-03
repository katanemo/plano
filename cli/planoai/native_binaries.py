import os
import platform
import subprocess
import sys
import tarfile
import tempfile

import planoai
from planoai.consts import (
    ENVOY_VERSION,
    PLANO_BIN_DIR,
    PLANO_PLUGINS_DIR,
    PLANO_RELEASE_BASE_URL,
)
from planoai.utils import find_repo_root, getLogger

log = getLogger(__name__)


def _get_platform_slug():
    """Return the platform slug for binary downloads."""
    system = platform.system().lower()
    machine = platform.machine().lower()

    mapping = {
        ("linux", "x86_64"): "linux-amd64",
        ("linux", "aarch64"): "linux-arm64",
        ("darwin", "arm64"): "darwin-arm64",
    }

    slug = mapping.get((system, machine))
    if slug is None:
        if system == "darwin" and machine == "x86_64":
            print(
                "Error: macOS x86_64 (Intel) is not supported. "
                "Pre-built binaries are only available for Apple Silicon (arm64)."
            )
            sys.exit(1)
        print(
            f"Error: Unsupported platform {system}/{machine}. "
            "Supported platforms: linux-amd64, linux-arm64, darwin-arm64"
        )
        sys.exit(1)

    return slug


def _download_file(url, dest):
    """Download a file from *url* to *dest* using curl."""
    try:
        subprocess.run(
            ["curl", "-fSL", "-o", dest, url],
            check=True,
        )
    except subprocess.CalledProcessError as e:
        print(f"Error downloading: {e}")
        print(f"URL: {url}")
        print("Please check your internet connection and try again.")
        sys.exit(1)


def ensure_envoy_binary():
    """Download Envoy binary if not already present or version changed. Returns path to binary."""
    envoy_path = os.path.join(PLANO_BIN_DIR, "envoy")
    version_path = os.path.join(PLANO_BIN_DIR, "envoy.version")

    if os.path.exists(envoy_path) and os.access(envoy_path, os.X_OK):
        # Check if cached binary matches the pinned version
        if os.path.exists(version_path):
            with open(version_path, "r") as f:
                cached_version = f.read().strip()
            if cached_version == ENVOY_VERSION:
                log.info(f"Envoy {ENVOY_VERSION} found at {envoy_path}")
                return envoy_path
            print(
                f"Envoy version changed ({cached_version} → {ENVOY_VERSION}), re-downloading..."
            )
        else:
            log.info(
                f"Envoy binary found at {envoy_path} (unknown version, re-downloading...)"
            )

    slug = _get_platform_slug()
    url = (
        f"https://github.com/tetratelabs/archive-envoy/releases/download/"
        f"{ENVOY_VERSION}/envoy-{ENVOY_VERSION}-{slug}.tar.xz"
    )

    os.makedirs(PLANO_BIN_DIR, exist_ok=True)

    print(f"Downloading Envoy {ENVOY_VERSION} for {slug}...")
    print(f"  URL: {url}")

    with tempfile.NamedTemporaryFile(suffix=".tar.xz", delete=False) as tmp:
        tmp_path = tmp.name

    try:
        _download_file(url, tmp_path)

        print("Extracting Envoy binary...")
        with tarfile.open(tmp_path, "r:xz") as tar:
            # Find the envoy binary inside the archive
            envoy_member = None
            for member in tar.getmembers():
                if member.name.endswith("/bin/envoy") or member.name == "bin/envoy":
                    envoy_member = member
                    break

            if envoy_member is None:
                print("Error: Could not find envoy binary in the downloaded archive.")
                print("Archive contents:")
                for member in tar.getmembers():
                    print(f"  {member.name}")
                sys.exit(1)

            # Extract just the binary
            f = tar.extractfile(envoy_member)
            if f is None:
                print("Error: Could not extract envoy binary from archive.")
                sys.exit(1)

            with open(envoy_path, "wb") as out:
                out.write(f.read())

        os.chmod(envoy_path, 0o755)
        with open(version_path, "w") as f:
            f.write(ENVOY_VERSION)
        print(f"Envoy {ENVOY_VERSION} installed at {envoy_path}")
        return envoy_path

    finally:
        if os.path.exists(tmp_path):
            os.unlink(tmp_path)


def ensure_wasm_plugins():
    """Download WASM plugins if not cached or version changed. Returns (prompt_gateway_path, llm_gateway_path)."""
    version = planoai.__version__
    version_path = os.path.join(PLANO_PLUGINS_DIR, "wasm.version")

    prompt_gw_path = os.path.join(PLANO_PLUGINS_DIR, "prompt_gateway.wasm")
    llm_gw_path = os.path.join(PLANO_PLUGINS_DIR, "llm_gateway.wasm")

    if os.path.exists(prompt_gw_path) and os.path.exists(llm_gw_path):
        if os.path.exists(version_path):
            with open(version_path, "r") as f:
                cached_version = f.read().strip()
            if cached_version == version:
                log.info(f"WASM plugins {version} found at {PLANO_PLUGINS_DIR}")
                return prompt_gw_path, llm_gw_path
            print(
                f"WASM plugins version changed ({cached_version} → {version}), re-downloading..."
            )
        else:
            log.info("WASM plugins found (unknown version, re-downloading...)")

    os.makedirs(PLANO_PLUGINS_DIR, exist_ok=True)

    for name, dest in [
        ("prompt_gateway.wasm", prompt_gw_path),
        ("llm_gateway.wasm", llm_gw_path),
    ]:
        url = f"{PLANO_RELEASE_BASE_URL}/{version}/{name}"
        print(f"Downloading {name} ({version})...")
        print(f"  URL: {url}")
        _download_file(url, dest)
        print(f"  Saved to {dest}")

    with open(version_path, "w") as f:
        f.write(version)

    return prompt_gw_path, llm_gw_path


def ensure_brightstaff_binary():
    """Download brightstaff binary if not cached or version changed. Returns path to binary."""
    version = planoai.__version__
    brightstaff_path = os.path.join(PLANO_BIN_DIR, "brightstaff")
    version_path = os.path.join(PLANO_BIN_DIR, "brightstaff.version")

    if os.path.exists(brightstaff_path) and os.access(brightstaff_path, os.X_OK):
        if os.path.exists(version_path):
            with open(version_path, "r") as f:
                cached_version = f.read().strip()
            if cached_version == version:
                log.info(f"brightstaff {version} found at {brightstaff_path}")
                return brightstaff_path
            print(
                f"brightstaff version changed ({cached_version} → {version}), re-downloading..."
            )
        else:
            log.info("brightstaff found (unknown version, re-downloading...)")

    slug = _get_platform_slug()
    filename = f"brightstaff-{slug}"
    url = f"{PLANO_RELEASE_BASE_URL}/{version}/{filename}"

    os.makedirs(PLANO_BIN_DIR, exist_ok=True)

    print(f"Downloading brightstaff ({version}) for {slug}...")
    print(f"  URL: {url}")
    _download_file(url, brightstaff_path)

    os.chmod(brightstaff_path, 0o755)
    with open(version_path, "w") as f:
        f.write(version)
    print(f"brightstaff {version} installed at {brightstaff_path}")
    return brightstaff_path


def find_wasm_plugins():
    """Find WASM plugin files built from source. Returns (prompt_gateway_path, llm_gateway_path)."""
    repo_root = find_repo_root()
    if not repo_root:
        print(
            "Error: Could not find repository root. "
            "Make sure you're inside the plano repository."
        )
        sys.exit(1)

    wasm_dir = os.path.join(repo_root, "crates", "target", "wasm32-wasip1", "release")
    prompt_gw = os.path.join(wasm_dir, "prompt_gateway.wasm")
    llm_gw = os.path.join(wasm_dir, "llm_gateway.wasm")

    missing = []
    if not os.path.exists(prompt_gw):
        missing.append("prompt_gateway.wasm")
    if not os.path.exists(llm_gw):
        missing.append("llm_gateway.wasm")

    if missing:
        print(f"Error: WASM plugins not found: {', '.join(missing)}")
        print(f"  Expected at: {wasm_dir}/")
        print("  Run 'planoai build --native' first to build them.")
        sys.exit(1)

    return prompt_gw, llm_gw


def find_brightstaff_binary():
    """Find the brightstaff binary built from source. Returns path."""
    repo_root = find_repo_root()
    if not repo_root:
        print(
            "Error: Could not find repository root. "
            "Make sure you're inside the plano repository."
        )
        sys.exit(1)

    brightstaff_path = os.path.join(
        repo_root, "crates", "target", "release", "brightstaff"
    )
    if not os.path.exists(brightstaff_path):
        print(f"Error: brightstaff binary not found at {brightstaff_path}")
        print("  Run 'planoai build --native' first to build it.")
        sys.exit(1)

    return brightstaff_path
