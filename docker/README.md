# Compiling and packaging the router

The following instructions will guide you through the process of compiling and packaging the router, **locally**.

> This process is also automated using GitHub Actions, see [workflow file](../.github/workflows/build.yaml).

## Binary

Make sure to install `zig` and `zigbuild` in your system:

```bash
brew install zig
cargo install --locked cargo-zigbuild
```

From the root of the project, run the following to build the `router` binaries (cross-build):

```bash
./docker/build-binary.sh
```

This will configure your local environment for cross-build of `linux/amd64` and `linux/arm64` binaries.

## Docker

To build a Docker image locally, ensure your system is using `containerd` as the container runtime, by following: https://docs.docker.com/desktop/features/containerd/#enable-the-containerd-image-store

Then, compile the binaries from the step above, and then run from the root:

```bash
./docker/build-docker-local.sh
```
