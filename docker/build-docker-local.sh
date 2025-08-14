#!/bin/bash
set -e

# Failed locally on macOS? see https://docs.docker.com/desktop/features/containerd/#enable-the-containerd-image-store
docker buildx build --platform linux/amd64,linux/arm64 -f ./docker/gateway.Dockerfile .
