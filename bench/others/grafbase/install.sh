#!/usr/bin/env bash
set -euo pipefail

# I took their install script and modified it to download the gateway binary
# to the current directory instead of /usr/local/bin

# https://github.com/grafbase/grafbase/releases
VERSION="0.40.0"

error() {
    echo -e "${Red}error${Color_Off}:" "$@" >&2
    exit 1
}


case $(uname -ms) in
'Darwin x86_64')
    # target=x86_64-apple-darwin
    target=aarch64-apple-darwin
    ;;
'Darwin arm64')
    target=aarch64-apple-darwin
    ;;
'Linux aarch64' | 'Linux arm64')
    target=aarch64-unknown-linux-musl
    ;;
'Linux x86_64' | *)
    target=x86_64-unknown-linux-musl
    ;;
esac

if [[ $target = darwin-x64 ]]; then
    # Is this process running in Rosetta?
    # redirect stderr to devnull to avoid error message when not running in Rosetta
    if [[ $(sysctl -n sysctl.proc_translated 2>/dev/null) = 1 ]]; then
        target=darwin-aarch64
        info "Your shell is running in Rosetta 2. Downloading grafbase-gateway for $target instead"
    fi
fi

gateway_uri=https://github.com/grafbase/grafbase/releases/download/gateway-$VERSION/grafbase-gateway-$target

exe="./grafbase-gateway"

echo "Downloading $gateway_uri"

curl --fail --location --progress-bar --output "$exe" "$gateway_uri" ||
    error "Failed to download grafbase gateway from \"$gateway_uri\""

chmod +x "$exe" ||
    error 'Failed to set permissions on grafbase gateway executable'


echo "success"
