#!/bin/bash
set -euo pipefail

# Plano CLI installer
# Usage: curl -fsSL https://raw.githubusercontent.com/katanemo/plano/main/install.sh | bash

REPO="katanemo/archgw"
BINARY_NAME="planoai"
INSTALL_DIR="${PLANO_INSTALL_DIR:-$HOME/.plano/bin}"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
CYAN='\033[0;36m'
DIM='\033[2m'
BOLD='\033[1m'
RESET='\033[0m'

info()  { echo -e "${GREEN}✓${RESET} $*"; }
error() { echo -e "${RED}✗${RESET} $*" >&2; }
dim()   { echo -e "${DIM}$*${RESET}"; }

# Detect platform
detect_platform() {
    local os arch
    os="$(uname -s)"
    arch="$(uname -m)"

    case "$os" in
        Linux)  os="linux" ;;
        Darwin) os="darwin" ;;
        *)      error "Unsupported OS: $os"; exit 1 ;;
    esac

    case "$arch" in
        x86_64)  arch="amd64" ;;
        aarch64) arch="arm64" ;;
        arm64)   arch="arm64" ;;
        *)       error "Unsupported architecture: $arch"; exit 1 ;;
    esac

    if [ "$os" = "darwin" ] && [ "$arch" = "amd64" ]; then
        error "macOS x86_64 (Intel) is not supported. Pre-built binaries are only available for Apple Silicon (arm64)."
        exit 1
    fi

    echo "${os}-${arch}"
}

# Get latest version from GitHub releases
get_latest_version() {
    local version
    version=$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
        | grep '"tag_name"' \
        | sed -E 's/.*"([^"]+)".*/\1/')
    echo "$version"
}

main() {
    echo -e "\n${BOLD}Plano CLI Installer${RESET}\n"

    # Detect platform
    local platform
    platform="$(detect_platform)"
    dim "  Platform: $platform"

    # Get version
    local version="${PLANO_VERSION:-}"
    if [ -z "$version" ]; then
        dim "  Fetching latest version..."
        version="$(get_latest_version)"
    fi
    if [ -z "$version" ]; then
        error "Could not determine version. Set PLANO_VERSION or check your internet connection."
        exit 1
    fi
    dim "  Version:  $version"

    # Download URL
    local url="https://github.com/${REPO}/releases/download/${version}/planoai-${platform}.gz"
    dim "  URL:      $url"
    echo ""

    # Create install directory
    mkdir -p "$INSTALL_DIR"

    # Download and extract
    local tmp_gz
    tmp_gz="$(mktemp)"
    echo -e "  ${DIM}Downloading planoai ${version}...${RESET}"

    if ! curl -fSL --progress-bar "$url" -o "$tmp_gz"; then
        error "Download failed. Check that version $version exists for platform $platform."
        rm -f "$tmp_gz"
        exit 1
    fi

    # Decompress
    echo -e "  ${DIM}Installing to ${INSTALL_DIR}/${BINARY_NAME}...${RESET}"
    gzip -d -c "$tmp_gz" > "${INSTALL_DIR}/${BINARY_NAME}"
    chmod +x "${INSTALL_DIR}/${BINARY_NAME}"
    rm -f "$tmp_gz"

    info "Installed planoai ${version} to ${INSTALL_DIR}/${BINARY_NAME}"

    # Check if install dir is in PATH
    if ! echo "$PATH" | tr ':' '\n' | grep -qx "$INSTALL_DIR"; then
        echo ""
        echo -e "  ${CYAN}Add to your PATH:${RESET}"
        local shell_name
        shell_name="$(basename "${SHELL:-/bin/bash}")"
        local rc_file
        case "$shell_name" in
            zsh)  rc_file="$HOME/.zshrc" ;;
            fish) rc_file="$HOME/.config/fish/config.fish" ;;
            *)    rc_file="$HOME/.bashrc" ;;
        esac

        if [ "$shell_name" = "fish" ]; then
            echo -e "    ${BOLD}set -gx PATH ${INSTALL_DIR} \$PATH${RESET}"
        else
            echo -e "    ${BOLD}export PATH=\"${INSTALL_DIR}:\$PATH\"${RESET}"
        fi
        echo -e "  ${DIM}Add this line to ${rc_file} to make it permanent.${RESET}"
    fi

    echo ""
    info "Run ${BOLD}planoai --help${RESET} to get started."
    echo ""
}

main "$@"
