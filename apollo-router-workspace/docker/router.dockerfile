# Runtime -> https://github.com/apollographql/router/blob/dev/dockerfiles/Dockerfile.router#L23
FROM debian:bookworm-slim AS runtime

RUN apt-get update
RUN apt-get -y install ca-certificates
RUN rm -rf /var/lib/apt/lists/*

LABEL org.opencontainers.image.title=$IMAGE_TITLE
LABEL org.opencontainers.image.version=$RELEASE
LABEL org.opencontainers.image.description=$IMAGE_DESCRIPTION
LABEL org.opencontainers.image.authors="The Guild"
LABEL org.opencontainers.image.vendor="Kamil Kisiela"
LABEL org.opencontainers.image.url="https://github.com/graphql-hive/router"
LABEL org.opencontainers.image.source="https://github.com/graphql-hive/router"

RUN mkdir -p /dist/config
RUN mkdir /dist/schema

# Copy in the required files from our build image
COPY --from=config --chown=root:root router.tar.gz /dist
COPY --from=router_pkg router.yaml /dist/config/router.yaml

# Extract the router binary
RUN tar -xvf /dist/router.tar.gz -C /dist
RUN rm /dist/router.tar.gz

WORKDIR /dist

ENV APOLLO_ROUTER_CONFIG_PATH="/dist/config/router.yaml"

ENTRYPOINT ["./router"]
