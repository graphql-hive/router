#!/bin/bash
# usage: cpu.sh <router-binary> [seconds] [label]
# Starts the router on :4000 against the bench subgraphs on :4200 (must be running),
# warms it, runs wrk, prints router CPU µs/request, req/s and RSS.
set -euo pipefail
REPO=$(cd "$(dirname "$0")/.." && pwd)
BIN=$1; DUR=${2:-20}; LABEL=${3:-$(basename $BIN)}
if lsof -iTCP:4000 -sTCP:LISTEN -t >/dev/null 2>&1; then echo ":4000 busy"; exit 1; fi
cd $REPO/bench/configs
ROUTER_CONFIG_FILE_PATH=${CFG:-$REPO/bench/configs/no-deduplication.config.yaml} LOG_LEVEL=error "$BIN" >/tmp/router-$$.log 2>&1 &
RPID=$!
trap 'kill $RPID 2>/dev/null || true' EXIT
for i in $(seq 1 100); do curl -s -o /dev/null localhost:4000/health && break; sleep 0.1; done
LPID=$(lsof -iTCP:4000 -sTCP:LISTEN -t)
[ "$LPID" = "$RPID" ] || { echo "pid mismatch $LPID vs $RPID"; exit 1; }
cd $REPO/bench
BENCH_OPERATION_FILE=operation.graphql BENCH_EXPECTED_RESPONSE_FILE=expected_response.json wrk -t4 -c50 -d3s -s wrk.lua http://127.0.0.1:4000/graphql >/dev/null 2>&1
t2s() { echo "$1" | awk -F'[:.]' '{ if (NF==3) print $1*60+$2+$3/100; else print $1*3600+$2*60+$3+$4/100 }'; }
C0=$(t2s $(ps -o time= -p $RPID | tr -d ' '))
OUT=$(BENCH_OPERATION_FILE=operation.graphql BENCH_EXPECTED_RESPONSE_FILE=expected_response.json wrk -t4 -c50 -d${DUR}s --latency -s wrk.lua http://127.0.0.1:4000/graphql 2>&1)
C1=$(t2s $(ps -o time= -p $RPID | tr -d ' '))
RSS=$(ps -o rss= -p $RPID | tr -d ' ')
REQS=$(echo "$OUT" | awk '/requests in/ {print $1}')
RPS=$(echo "$OUT" | awk '/Requests\/sec/ {print $2}')
P99=$(echo "$OUT" | awk '$1=="99%" {print $2}')
FAIL=$(echo "$OUT" | grep -i -E "VALIDATION|non-2xx" | tr '\n' ' ')
echo "$LABEL cpu_us_per_req=$(echo "($C1-$C0)*1000000/$REQS" | bc -l | cut -c1-6) rps=$RPS p99=$P99 rss_kb=$RSS reqs=$REQS $FAIL"
