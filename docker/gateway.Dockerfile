# syntax=docker/dockerfile:1.11

FROM gcr.io/distroless/cc-debian12
ARG TARGETARCH

WORKDIR /app
COPY ./target/linux/${TARGETARCH}/gateway ./
EXPOSE 4000

CMD ["./gateway", "/app/config/supergraph.graphql"]
