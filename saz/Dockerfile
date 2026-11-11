FROM debian:trixie-slim

RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*

COPY config /app/config
COPY hive-router /usr/local/bin/hive-router

ENV ROUTER_CONFIG_FILE_PATH=/app/config/router.yaml

WORKDIR /app

USER nobody

CMD ["hive-router"]
