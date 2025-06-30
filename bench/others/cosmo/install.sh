#!/bin/sh

set -u

# Cosmo has no install.sh script, so I took the install.sh script from the router gateway
# and modified it to download the Cosmo binary instead of the router binary

BINARY_DOWNLOAD_PREFIX="https://github.com/wundergraph/cosmo/releases/download"

# https://github.com/wundergraph/cosmo/releases
DOWNLOAD_VERSION="0.222.1"

download_binary() {
    downloader --check
    need_cmd mktemp
    need_cmd chmod
    need_cmd mkdir
    need_cmd rm
    need_cmd rmdir
    need_cmd tar
    need_cmd which
    need_cmd dirname
    need_cmd awk
    need_cmd cut


    get_architecture || return 1
    _arch="$RETVAL"
    assert_nz "$_arch" "arch"


    _tardir="router@$DOWNLOAD_VERSION/router-router@$DOWNLOAD_VERSION-${_arch}"
    _url="$BINARY_DOWNLOAD_PREFIX/${_tardir}.tar.gz"
    _dir="$(mktemp -d 2>/dev/null || ensure mktemp -d -t cosmo)"
    _file="$_dir/input.tar.gz"
    _router="$_dir/router"

    say "Downloading cosmo from $_url ..." 1>&2

    ensure mkdir -p "$_dir"
    downloader "$_url" "$_file"
    if [ $? != 0 ]; then
      say "Failed to download $_url"
      exit 1
    fi

    ensure tar xf "$_file" -C "$_dir"

    outfile="./cosmo"

    say "Moving $_router to $outfile ..."
    mv "$_router" "$outfile"

    _retval=$?

    say ""

    ignore rm -rf "$_dir"

    return "$_retval"
}

get_architecture() {
    case $(uname -ms) in
    'Darwin x86_64')
        target=darwin-amd64
        ;;
    'Darwin arm64')
        target=darwin-arm64
        ;;
    'Linux arm64')
        target=linux-arm64
        ;;
    'Linux aarch64')
        target=linux-amd64
        ;;
    'Linux x86_64' | *)
        target=linux-amd64
        ;;
    esac

    RETVAL="$target"
}

say() {
    green=$(tput setaf 2 2>/dev/null || echo '')
    reset=$(tput sgr0 2>/dev/null || echo '')
    echo "$1"
}

err() {
    red=$(tput setaf 1 2>/dev/null || echo '')
    reset=$(tput sgr0 2>/dev/null || echo '')
    say "${red}ERROR${reset}: $1" >&2
    exit 1
}

need_cmd() {
    if ! check_cmd "$1"
    then err "Installation halted. Reason: [command not found '$1' - please install this command]"
    fi
}

check_cmd() {
    command -v "$1" > /dev/null 2>&1
    return $?
}

need_ok() {
    if [ $? != 0 ]; then err "$1"; fi
}

assert_nz() {
    if [ -z "$1" ]; then err "assert_nz $2"; fi
}

# Run a command that should never fail. If the command fails execution
# will immediately terminate with an error showing the failing
# command.
ensure() {
    "$@"
    need_ok "command failed: $*"
}

# This is just for indicating that commands' results are being
# intentionally ignored. Usually, because it's being executed
# as part of error handling.
ignore() {
    "$@"
}

# This wraps curl or wget. Try curl first, if not installed,
# use wget instead.
downloader() {
    if check_cmd curl
    then _dld=curl
    elif check_cmd wget
    then _dld=wget
    else _dld='curl or wget' # to be used in error message of need_cmd
    fi

    if [ "$1" = --check ]
    then need_cmd "$_dld"
    elif [ "$_dld" = curl ]
    then curl -sSfL "$1" -o "$2"
    elif [ "$_dld" = wget ]
    then wget "$1" -O "$2"
    else err "Unknown downloader"   # should not reach here
    fi
}

download_binary "$@" || exit 1
