# syntax=docker/dockerfile:1.17

FROM gcr.io/distroless/cc-debian12
ARG TARGETARCH

WORKDIR /app
COPY --chmod=755 ./target/linux/${TARGETARCH}/hive_router ./
COPY --chmod=755 ./docker/entrypoint.sh ./
EXPOSE 4000

ENV HIVE_ROUTER_CONFIG=/app/config/supergraph.graphql

CMD ["./entrypoint.sh"]
