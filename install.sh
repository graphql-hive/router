#!/bin/sh
set -e

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

info() {
    echo "\033[34m[INFO]\033[0m $1"
}

error() {
    echo "\033[31m[ERROR]\033[0m $1" >&2
    exit 1
}

check_tool() {
    if ! command -v "$1" >/dev/null 2>&1; then
        error "'$1' is required but it's not installed. Please install it to continue."
    fi
}

banner() {
  echo "       @@@@@@@@@@@@@                                                                                                            "
  echo "     @@                                                                                                                         "
  echo "   @@@   #++++#      +@@          @@@     @@@  @@                        @@@@@@@@@                       @@#                    "
  echo "   @@@  @@@@@@@@@     @@@         @@@     =@@                            @@     =@@                      @@@                    "
  echo "   @@@  @@@      @@   @@@         @@@      @@  @@ @@#   @@@ @@@@@@@      @@      @@  @@@@@@@   @@    @@ @@@@@+ @@@@@@@  @@#@@@  "
  echo "   @@@  @@@      @@@  @@@         @@@@@@@@@@@  @@  @@   @@ @@     @@     @@@@@@@@@  @@#    @@  @@    @@  @@%  @@     @@ @@@     "
  echo "   @@@   @@      @@@  @@@         @@@      @@  @@   @@ @@  @@@@@@@@@     @@+    @@@ @@      @@ @@    @@  @@@  @@@@@@@@@ @@      "
  echo "   @@@     @@@@@@@@@  @@@         @@@     =@@  @@   @@ @@  @@     @@     @@*     @@ @@@    @@  @@    @@  @@%  @@     @@ @@      "
  echo "     @                @@          @@@     @@@  @@    @@@    @@@@@@@      @@@     @@  :@@@@@@   @@@@@@@@   @@@* #@@@@@@  @@=     "
  echo "       @@@@@@@@@@@@@@@                                                                                                          "
  echo "        @@@@@@@@@@@@                                                                                                            "
}

detect_arch() {
    OS_TYPE=$(uname -s)
    ARCH=$(uname -m)

    info "Detecting operating system and architecture..."

    case $OS_TYPE in
        Linux)
            OS="linux"
            ;;
        Darwin)
            OS="macos"
            ;;
        *)
            error "Unsupported operating system: '$OS_TYPE'. You may use Hive Router using Docker by following the instructions at https://github.com/graphql-hive/router#docker"
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
            error "Unsupported architecture: '$ARCH'. You may use Hive Router using Docker by following the instructions at https://github.com/graphql-hive/router#docker"
            ;;
    esac
    info "System detected: ${OS}-${ARCH}"
}

get_version() {
    if [ -n "$1" ]; then
        VERSION="$1"
        info "Installing specified version: $VERSION"
    else
        info "No version specified. Fetching the latest version from crates.io index..."

        # Uses index.crates.io which is more reliable than the rate-limited Crates API
        # Path structure: /first_two_chars/next_two_chars/full_name
        CRATE_FIRST_TWO=$(echo "${CARGO_PKG_NAME}" | cut -c1-2)
        CRATE_NEXT_TWO=$(echo "${CARGO_PKG_NAME}" | cut -c3-4)

        INDEX_URL="https://index.crates.io/${CRATE_FIRST_TWO}/${CRATE_NEXT_TWO}/${CARGO_PKG_NAME}"

        # The index returns newline-delimited JSON; extract the version from the last line
        VERSION="v$(curl -sL "$INDEX_URL" | tail -1 | grep -o '"vers":"[^"]*"' | sed 's/"vers":"\([^"]*\)"/\1/')"

        if [ -z "$VERSION" ] || [ "$VERSION" = "v" ]; then
            error "Could not determine the latest version from crates.io index. Please check the crate name or specify a version manually."
        fi
        info "Latest version found: $VERSION"
    fi
}

download_and_install() {
    ASSET_NAME="${BINARY_NAME}_${OS}_${ARCH}"
    DOWNLOAD_URL="https://github.com/${GH_OWNER}/${GH_REPO}/releases/download/router%2F${VERSION}/${ASSET_NAME}"

    info "Downloading binary from: ${DOWNLOAD_URL}"

    # -f: Fail silently on server errors (like 404)
    # -L: Follow redirects
    if ! curl -fL -o "./${BINARY_NAME}" "${DOWNLOAD_URL}"; then
        error "Download failed. Please check if the version '$VERSION' and architecture '$OS_TYPE/$ARCH' exist for this release."
    fi

    chmod +x "./${BINARY_NAME}"

    info "✅ Successfully installed '${BINARY_NAME}' to the current directory."
    info "You can now run it with: ./${BINARY_NAME}"
    info ""
    info "Getting started instructions: https://github.com/graphql-hive/router#try-it-out"
    info "Config file reference: https://github.com/graphql-hive/router/blob/main/docs/README.md"
}

main() {
    check_tool "curl"
    check_tool "grep"
    check_tool "sed"
    check_tool "uname"

    banner

    detect_arch
    get_version "$1"
    download_and_install
}

main "$@"
