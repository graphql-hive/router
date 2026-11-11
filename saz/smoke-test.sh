#!/usr/bin/env bash
# Smoke test against a running router built with the logger plugin.
# Usage: saz/smoke-test.sh '<command that prints the router logs>'
#   e.g. saz/smoke-test.sh 'docker logs router'
set -euo pipefail

URL=${ROUTER_URL:-http://localhost:8080}
LOGS_CMD=$1
N=50

for _ in $(seq 1 60); do curl -sf -o /dev/null "$URL/readiness" && break; sleep 1; done
curl -sf -o /dev/null "$URL/health"
curl -sf -o /dev/null "$URL/readiness"
echo "health + readiness OK"

curl -sf -H 'Content-Type: application/json' -d '{"query":"query Ping { __schema { queryType { name } } }"}' "$URL/" \
  | grep -q '"queryType":{"name":"Query"}'
echo "query OK (introspection, no subgraph needed)"

# Concurrent requests: each log line must carry its own request's data (logger concurrency regression).
for i in $(seq 1 $N); do
  curl -sf -o /dev/null -H 'Content-Type: application/json' -H "x-correlation-id: req-$i" \
    -d "{\"query\":\"query Ping$i { __schema { queryType { name } } }\"}" "$URL/" &
done
wait
sleep 1

logs=$(eval "$LOGS_CMD" 2>&1)
lines=$(grep '"correlationId":"req-' <<<"$logs" || true)
count=$(grep -c . <<<"$lines" || true)
bad=$(jq -s '[.[] | select((.operationName | ltrimstr("Ping")) != (.extra.correlationId | ltrimstr("req-")))] | length' <<<"$lines")
echo "logger lines: $count/$N, mismatched: $bad"
if [ "$count" -ne "$N" ] || [ "$bad" -ne 0 ]; then
  echo "$logs" | tail -50
  exit 1
fi
echo "smoke test passed"
