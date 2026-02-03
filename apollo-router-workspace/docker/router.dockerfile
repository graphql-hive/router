# Based on https://github.com/apollographql/router/blob/dev/dockerfiles/Dockerfile.router#L23
FROM debian:bookworm-slim AS runtime
ARG DEBUG_IMAGE=false
ARG REPO_URL=https://github.com/graphql-hive/router
ARG BASE_VERSION

# Add a user to run the router as
RUN useradd -m router

WORKDIR /dist

COPY --from=config --chown=root:root --chmod=755 ./target/release/router /dist

# Update apt and install ca-certificates
RUN \
  apt-get update -y \
  && apt-get install -y \
    ca-certificates

# If debug image, install heaptrack and make a data directory
RUN \
  if [ "${DEBUG_IMAGE}" = "true" ]; then \
    apt-get install -y heaptrack && \
    mkdir data && \
    chown router data; \
  fi

# Clean up apt lists
RUN rm -rf /var/lib/apt/lists/*

# Make directories for config and schema
RUN mkdir config schema

# Copy configuration for docker image
COPY --from=router_pkg router.yaml /dist/config/router.yaml

LABEL org.opencontainers.image.title="graphql-hive/apollo-router"
LABEL org.opencontainers.image.description="Apollo Router for GraphQL Hive."
LABEL org.opencontainers.image.authors="The Guild ${REPO_URL}"
LABEL org.opencontainers.image.source="${REPO_URL}"
LABEL org.opencontainers.image.version="${BASE_VERSION}"

ENV APOLLO_ROUTER_CONFIG_PATH="/dist/config/router.yaml"

# Create a wrapper script to run the router, use exec to ensure signals are handled correctly
RUN \
  echo '#!/bin/bash \
\nset -e \
\n \
\nif [ -f "/usr/bin/heaptrack" ]; then \
\n    exec heaptrack -o /dist/data/$(hostname)/router_heaptrack  /dist/router "$@" \
\nelse \
\n    exec /dist/router "$@" \
\nfi \
' > /dist/router_wrapper.sh

# Make sure we can run our wrapper
RUN chmod 755 /dist/router_wrapper.sh

USER router

# Default executable is the wrapper script
ENTRYPOINT ["/dist/router_wrapper.sh"]