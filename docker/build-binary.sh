#!/bin/bash
set -e

export BINARY_NAME=hive_router

# Linux ARM (aarch64)
rm -rf target/linux/arm64
rustup target add aarch64-unknown-linux-gnu
cargo zigbuild --release --target aarch64-unknown-linux-gnu.2.28
mkdir -p target/linux/arm64
cp target/aarch64-unknown-linux-gnu/release/$BINARY_NAME target/linux/arm64/

# Linux AMD (x86_64):
rm -rf target/linux/amd64
rustup target add x86_64-unknown-linux-gnu
cargo zigbuild --release --target x86_64-unknown-linux-gnu.2.28
mkdir -p target/linux/amd64
cp target/x86_64-unknown-linux-gnu/release/$BINARY_NAME target/linux/amd64/
