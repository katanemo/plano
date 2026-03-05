import gzip
import os
import platform
import shutil
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


def _find_local_wasm_plugins():
    """Check for WASM plugins built from source. Returns (prompt_gw, llm_gw) or None."""
    repo_root = find_repo_root()
    if not repo_root:
        return None
    wasm_dir = os.path.join(repo_root, "crates", "target", "wasm32-wasip1", "release")
    prompt_gw = os.path.join(wasm_dir, "prompt_gateway.wasm")
    llm_gw = os.path.join(wasm_dir, "llm_gateway.wasm")
    if os.path.exists(prompt_gw) and os.path.exists(llm_gw):
        return prompt_gw, llm_gw
    return None


def _find_local_brightstaff():
    """Check for brightstaff binary built from source. Returns path or None."""
    repo_root = find_repo_root()
    if not repo_root:
        return None
    path = os.path.join(repo_root, "crates", "target", "release", "brightstaff")
    if os.path.exists(path) and os.access(path, os.X_OK):
        return path
    return None


def ensure_wasm_plugins():
    """Find or download WASM plugins. Checks: local build → cached download → fresh download."""
    # 1. Local source build (inside repo)
    local = _find_local_wasm_plugins()
    if local:
        log.info(f"Using locally-built WASM plugins: {local[0]}")
        return local

    # 2. Cached download
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

    # 3. Download from GitHub releases (gzipped)
    os.makedirs(PLANO_PLUGINS_DIR, exist_ok=True)

    for name, dest in [
        ("prompt_gateway.wasm", prompt_gw_path),
        ("llm_gateway.wasm", llm_gw_path),
    ]:
        gz_name = f"{name}.gz"
        url = f"{PLANO_RELEASE_BASE_URL}/{version}/{gz_name}"
        print(f"Downloading {gz_name} ({version})...")
        print(f"  URL: {url}")
        gz_dest = dest + ".gz"
        _download_file(url, gz_dest)
        with gzip.open(gz_dest, "rb") as f_in, open(dest, "wb") as f_out:
            shutil.copyfileobj(f_in, f_out)
        os.unlink(gz_dest)
        print(f"  Saved to {dest}")

    with open(version_path, "w") as f:
        f.write(version)

    return prompt_gw_path, llm_gw_path


def ensure_brightstaff_binary():
    """Find or download brightstaff binary. Checks: local build → cached download → fresh download."""
    # 1. Local source build (inside repo)
    local = _find_local_brightstaff()
    if local:
        log.info(f"Using locally-built brightstaff: {local}")
        return local

    # 2. Cached download
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

    # 3. Download from GitHub releases (gzipped)
    slug = _get_platform_slug()
    filename = f"brightstaff-{slug}.gz"
    url = f"{PLANO_RELEASE_BASE_URL}/{version}/{filename}"

    os.makedirs(PLANO_BIN_DIR, exist_ok=True)

    print(f"Downloading brightstaff ({version}) for {slug}...")
    print(f"  URL: {url}")
    gz_path = brightstaff_path + ".gz"
    _download_file(url, gz_path)
    with gzip.open(gz_path, "rb") as f_in, open(brightstaff_path, "wb") as f_out:
        shutil.copyfileobj(f_in, f_out)
    os.unlink(gz_path)

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
        print("  Run 'planoai build' first to build them.")
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
        print("  Run 'planoai build' first to build it.")
        sys.exit(1)

    return brightstaff_path
