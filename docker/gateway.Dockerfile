# syntax=docker/dockerfile:1.11

ARG RUST_TOOLCHAIN_VERSION="1.89.0"
ARG TARGETPLATFORM="local"

# Builder
FROM rust:${RUST_TOOLCHAIN_VERSION}-slim AS builder

RUN --mount=type=cache,target=/usr/local/cargo/registry,id=${TARGETPLATFORM} \
  cargo install cargo-strip

WORKDIR /root

COPY Cargo.toml Cargo.lock ./
COPY lib ./lib
COPY bin ./bin
COPY bench ./bench

RUN --mount=type=cache,target=/usr/local/cargo/registry,id=${TARGETPLATFORM} --mount=type=cache,target=/root/target,id=${TARGETPLATFORM} \
  cargo fetch --locked

RUN --mount=type=cache,target=/usr/local/cargo/registry,id=${TARGETPLATFORM} --mount=type=cache,target=/root/target,id=${TARGETPLATFORM} \
  cargo build --package gateway --release && cargo strip && mv /root/target/release/gateway /root


# Runtime
FROM gcr.io/distroless/cc-debian12

WORKDIR /app

COPY --from=builder /root/gateway ./

EXPOSE 4000

CMD ["./gateway", "/app/config/supergraph.graphql"]
