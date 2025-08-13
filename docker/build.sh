#!/bin/bash
set -e

export RUST_TOOLCHAIN_VERSION=$(grep 'channel' rust-toolchain.toml | sed -E 's/.*"([^"]+)".*/\1/')
export DOCKER_BUILDKIT=1

echo "Using Rust toolchain version: $RUST_TOOLCHAIN_VERSION"

docker build --progress=plain -f docker/gateway.Dockerfile --build-arg RUST_TOOLCHAIN_VERSION=$RUST_TOOLCHAIN_VERSION .
