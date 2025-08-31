#!/bin/bash
set -e

if [[ -n "$HIVE_ROUTER_CONFIG" && -f "$HIVE_ROUTER_CONFIG" ]]; then
  export HIVE_SUPERGRAPH_SOURCE=file
  export HIVE_SUPERGRAPH_PATH=/app/config/supergraph.graphql
fi

./hive_router
