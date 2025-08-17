#!/bin/bash
set -e

export BINARY_NAME=gateway

echo "Cleaning up old builds..."
rm -rf target/linux

echo "Creating platform-specific directories..."
mkdir -p target/linux/amd64
mkdir -p target/linux/arm64

# Linux ARM (aarch64)
echo "Building for Linux ARM (aarch64)"
rustup target add aarch64-unknown-linux-gnu
cargo zigbuild --release --target aarch64-unknown-linux-gnu.2.17
echo "Linux ARM binary: target/aarch64-unknown-linux-gnu/release/$BINARY_NAME"
cp target/aarch64-unknown-linux-gnu/release/$BINARY_NAME target/linux/arm64/

# Linux AMD (x86_64):
echo "Building for Linux AMD (x86_64)"
rustup target add x86_64-unknown-linux-gnu
cargo zigbuild --release --target x86_64-unknown-linux-gnu.2.17
echo "Linux AMD binary: target/x86_64-unknown-linux-gnu/release/$BINARY_NAME"
cp target/x86_64-unknown-linux-gnu/release/$BINARY_NAME target/linux/amd64/
