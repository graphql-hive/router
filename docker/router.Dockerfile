# syntax=docker/dockerfile:1.17

FROM gcr.io/distroless/cc-debian12
ARG TARGETARCH

WORKDIR /app
COPY --chmod=755 ./target/linux/${TARGETARCH}/hive_router ./
EXPOSE 4000

CMD ["./hive_router", "/app/config/supergraph.graphql"]
