#!/usr/bin/env bash
set -e

# Color support: check NO_COLOR or non-TTY (stdout)
if [ -n "${NO_COLOR+x}" ] || [ ! -t 1 ]; then
    BLUE=''
    GREEN=''
    RED=''
    BOLD=''
    DIM=''
    NC=''
else
    BLUE='\033[0;34m'
    GREEN='\033[0;32m'
    RED='\033[0;31m'
    BOLD='\033[1m'
    DIM='\033[2m'
    NC='\033[0m'
fi

# ==============================================================================
# Hive Router
#
# Usage:
#   - Latest version: curl -sL https://your-domain.com/install | sh
#   - Specific version: curl -sL https://your-domain.com/install | sh -s v0.0.2
# ==============================================================================

GH_OWNER="graphql-hive"
GH_REPO="router"
BINARY_NAME="hive_router"
CARGO_PKG_NAME="hive-router"

print_step() {
    echo -e "${BLUE}▸${NC} $1"
}

print_success() {
    # Move up one line and clear it
    echo -ne "\033[1A\033[2K\r"
    echo -e "${GREEN}✓${NC} $1"
}

print_error() {
    echo -e "${RED}✗${NC} $1"
}

check_tool() {
    if ! command -v "$1" >/dev/null 2>&1; then
        print_error "'$1' is required but it's not installed. Please install it to continue."
        exit 1
    fi
}

detect_arch() {
    OS_TYPE=$(uname -s)
    ARCH=$(uname -m)

    print_step "Detecting system architecture..."

    case $OS_TYPE in
        Linux)
            OS="linux"
            ;;
        Darwin)
            OS="macos"
            ;;
        *)
            print_error "Unsupported operating system: '$OS_TYPE'. You may use Hive Router using Docker by following the instructions at https://the-guild.dev/graphql/hive/docs/router/getting-started"
            exit 1
            ;;
    esac

    case $ARCH in
        x86_64 | amd64)
            ARCH="amd64"
            ;;
        aarch64 | arm64)
            ARCH="arm64"
            ;;
        *)
            print_error "Unsupported architecture: '$ARCH'. You may use Hive Router using Docker by following the instructions at https://the-guild.dev/graphql/hive/docs/router/getting-started"
            exit 1
            ;;
    esac
    print_success "Detected ${OS}-${ARCH}"
}

get_version() {
    print_step "Resolving version..."

    if [ -n "$1" ]; then
        VERSION="$1"
        print_success "Using specified version: $VERSION"
    else
        # Uses index.crates.io which is more reliable than the rate-limited Crates API
        # Path structure: /first_two_chars/next_two_chars/full_name
        CRATE_FIRST_TWO=$(echo "${CARGO_PKG_NAME}" | cut -c1-2)
        CRATE_NEXT_TWO=$(echo "${CARGO_PKG_NAME}" | cut -c3-4)

        INDEX_URL="https://index.crates.io/${CRATE_FIRST_TWO}/${CRATE_NEXT_TWO}/${CARGO_PKG_NAME}"

        # The index returns newline-delimited JSON; extract the version from the last line
        VERSION="v$(curl -sL "$INDEX_URL" | tail -1 | grep -o '"vers":"[^"]*"' | sed 's/"vers":"\([^"]*\)"/\1/')"

        if [ -z "$VERSION" ] || [ "$VERSION" = "v" ]; then
            print_error "Could not determine the latest version from crates.io index. Please check the crate name or specify a version manually."
            exit 1
        fi
        print_success "Latest version found: $VERSION"
    fi
}

download_binary() {
    ASSET_NAME="${BINARY_NAME}_${OS}_${ARCH}"
    DOWNLOAD_URL="https://github.com/${GH_OWNER}/${GH_REPO}/releases/download/hive-router%2F${VERSION}/${ASSET_NAME}"

    print_step "Downloading Hive Router binary..."
    echo -e "${DIM}  Download URL: ${DOWNLOAD_URL}${NC}"

    # -f: Fail silently on server errors (like 404)
    # -L: Follow redirects
    if ! curl -fL --progress-bar -o "./${BINARY_NAME}" "${DOWNLOAD_URL}"; then
        print_error "Download failed. Please check if the version '$VERSION' and architecture '$OS_TYPE/$ARCH' exist for this release."
        exit 1
    fi

    if [ -t 1 ]; then
        echo -ne "\033[1A\033[2K\r"
        echo -ne "\033[1A\033[2K\r"
        echo -ne "\033[1A\033[2K\r"
    fi

    print_success "Binary downloaded"
}

install_binary() {
    print_step "Finalizing installation..."
    chmod +x "./${BINARY_NAME}"
    print_success "Binary made executable"
}

main() {
    echo ""
    echo -e "${BOLD}Hive Router Installer${NC}"
    echo ""

    check_tool "curl"
    check_tool "grep"
    check_tool "sed"
    check_tool "uname"

    detect_arch
    get_version "$1"
    download_binary
    install_binary

    echo ""
    echo -e "${BOLD}${GREEN}✨ Installation Complete!${NC}"
    echo ""
    echo -e "${BOLD}Start using Hive Router:${NC}"
    echo -e "   ${BOLD}${BINARY_NAME}${NC}"
    echo ""
    echo -e "${BOLD}Documentation:${NC}"
    echo -e "   https://the-guild.dev/graphql/hive/docs/router"
    echo ""
}

main "$@"
