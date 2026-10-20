#!/bin/bash

# Heap-profiles the router (dhat) while it serves the benchmark query against the benchmark subgraphs,
# then converts the result to pprof so `go tool pprof` can read it.
#
#   ./bench/memory-profile.sh
#   REQUESTS=5000 CONCURRENCY=20 ./bench/memory-profile.sh
#   ROUTER_CONFIG=bench/configs/no-deduplication.config.yaml ./bench/memory-profile.sh
#
# Output (bench/results/memory): heap.pb.gz, dhat-heap.json, logs.
# Copy heap.pb.gz to baseline.pb.gz and every next run also prints the diff against it.

set -euo pipefail

for cmd in hey jq curl go python3; do
  command -v "$cmd" >/dev/null || { echo "$cmd is required"; exit 1; }
done

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

OUT_DIR="$REPO_ROOT/bench/results/memory"
ROUTER_CONFIG="$REPO_ROOT/${ROUTER_CONFIG:-bench/configs/default.config.yaml}"
# fixed request count, so totals of two runs can be compared
REQUESTS="${REQUESTS:-1000}"
CONCURRENCY="${CONCURRENCY:-10}"
# not the usual 4000/4200, so it can run next to a regular benchmark
ROUTER_PORT="${ROUTER_PORT:-4400}"
SUBGRAPHS_PORT="${SUBGRAPHS_PORT:-4401}"
ROUTER_URL="http://127.0.0.1:$ROUTER_PORT/graphql"

step() {
  echo "==> [$(date +%T)] $*"
}

step "building subgraphs (release)"
cargo build --release -p subgraphs
step "building router (memprof, dhat-heap)"
cargo build --profile memprof -p hive-router --bin hive_router --features dhat-heap

for port in "$ROUTER_PORT" "$SUBGRAPHS_PORT"; do
  if lsof -iTCP:"$port" -sTCP:LISTEN >/dev/null 2>&1; then
    echo "port $port is taken, set ROUTER_PORT / SUBGRAPHS_PORT"
    exit 1
  fi
done

mkdir -p "$OUT_DIR"
rm -f "$OUT_DIR/dhat-heap.json" "$OUT_DIR/heap.pb.gz"
sed "s#http://0.0.0.0:4200/#http://127.0.0.1:$SUBGRAPHS_PORT/#g" bench/supergraph.graphql >"$OUT_DIR/supergraph.graphql"
jq -n --rawfile query bench/operation.graphql '{query: $query}' >"$OUT_DIR/body.json"

SUBGRAPHS_PID=""
ROUTER_PID=""
cleanup() {
  [ -n "$ROUTER_PID" ] && kill "$ROUTER_PID" 2>/dev/null || true
  [ -n "$SUBGRAPHS_PID" ] && kill "$SUBGRAPHS_PID" 2>/dev/null || true
}
trap cleanup EXIT

# waits for $2 to answer, while making sure it's our process ($1) answering
wait_for() {
  for _ in $(seq 1 60); do
    kill -0 "$1" 2>/dev/null || { echo "process exited, check the logs in $OUT_DIR"; exit 1; }
    curl -sf -o /dev/null "$2" && return 0
    sleep 0.5
  done
  echo "timed out waiting for $2"
  exit 1
}

step "starting subgraphs on :$SUBGRAPHS_PORT"
PORT="$SUBGRAPHS_PORT" ./target/release/subgraphs >"$OUT_DIR/subgraphs.log" 2>&1 &
SUBGRAPHS_PID=$!
wait_for "$SUBGRAPHS_PID" "http://127.0.0.1:$SUBGRAPHS_PORT/health"

step "starting router on :$ROUTER_PORT"
# dhat writes dhat-heap.json into the cwd
(cd "$OUT_DIR" && exec env \
  ROUTER_CONFIG_FILE_PATH="$ROUTER_CONFIG" \
  SUPERGRAPH_FILE_PATH="$OUT_DIR/supergraph.graphql" \
  PORT="$ROUTER_PORT" \
  "$REPO_ROOT/target/memprof/hive_router") >"$OUT_DIR/router.log" 2>&1 &
ROUTER_PID=$!
wait_for "$ROUTER_PID" "http://127.0.0.1:$ROUTER_PORT/health"

step "checking the response against bench/expected_response.json"
# make sure we profile the real thing, not an error path
if ! diff -q \
  <(curl -sf -H 'content-type: application/json' --data @"$OUT_DIR/body.json" "$ROUTER_URL" | jq -S .) \
  <(jq -S . bench/expected_response.json) >/dev/null; then
  echo "router response does not match bench/expected_response.json"
  exit 1
fi

step "sending $REQUESTS requests ($CONCURRENCY concurrent)"
hey -n "$REQUESTS" -c "$CONCURRENCY" -disable-compression -m POST -T application/json -D "$OUT_DIR/body.json" "$ROUTER_URL" >"$OUT_DIR/hey.txt"
grep -E 'Requests/sec|\[200\]' "$OUT_DIR/hey.txt" || true
# anything but all-200 means the profile is mostly error paths
if ! grep -qE "\[200\][[:space:]]+$REQUESTS responses" "$OUT_DIR/hey.txt"; then
  sed -n '/Status code/,$p' "$OUT_DIR/hey.txt"
  exit 1
fi

step "stopping router, waiting for dhat to write its report"
# graceful shutdown is what makes dhat write its report
kill -INT "$ROUTER_PID"
wait "$ROUTER_PID" || true
ROUTER_PID=""
grep '^dhat:' "$OUT_DIR/router.log" || true

step "converting dhat-heap.json to pprof"
python3 "$SCRIPT_DIR/dhat-to-pprof.py" "$OUT_DIR/dhat-heap.json" "$OUT_DIR/heap.pb.gz"

step "top allocation sites (alloc_space)"
go tool pprof -top -nodecount=25 -sample_index=alloc_space "$OUT_DIR/heap.pb.gz" 2>/dev/null
if [ -f "$OUT_DIR/baseline.pb.gz" ]; then
  step "diff against baseline.pb.gz"
  go tool pprof -top -nodecount=25 -sample_index=alloc_space -diff_base="$OUT_DIR/baseline.pb.gz" "$OUT_DIR/heap.pb.gz" 2>/dev/null
fi

echo
echo "Explore: go tool pprof -http=:8080 $OUT_DIR/heap.pb.gz"
