#!/bin/bash

set -euo pipefail

require_cmd() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "$1 could not be found. Please install $1 to run this benchmark."
    exit 1
  }
}

duration_to_seconds() {
  local value="$1"

  if [[ "$value" =~ ^([0-9]+)$ ]]; then
    echo "${BASH_REMATCH[1]}"
    return 0
  fi

  if [[ "$value" =~ ^([0-9]+)([smh])$ ]]; then
    local amount="${BASH_REMATCH[1]}"
    case "${BASH_REMATCH[2]}" in
      s) echo "$amount" ;;
      m) echo $((amount * 60)) ;;
      h) echo $((amount * 3600)) ;;
      *) return 1 ;;
    esac
    return 0
  fi

  return 1
}

resolve_summary_path() {
  if [ -z "${SUMMARY_PATH:-}" ]; then
    echo "$SCRIPT_DIR/results/pr"
  elif [[ "$SUMMARY_PATH" = /* ]]; then
    echo "$SUMMARY_PATH"
  elif [[ "$SUMMARY_PATH" == bench/* ]] || [[ "$SUMMARY_PATH" == ./bench/* ]]; then
    echo "$REPO_ROOT/${SUMMARY_PATH#./}"
  else
    echo "$SCRIPT_DIR/$SUMMARY_PATH"
  fi
}

show_progress() {
  local wrk_pid="$1"
  local total_seconds="$2"
  local start_time
  local now
  local elapsed
  local remaining

  start_time=$(date +%s)
  while kill -0 "$wrk_pid" 2>/dev/null; do
    if [ -n "$total_seconds" ]; then
      now=$(date +%s)
      elapsed=$((now - start_time))
      remaining=$((total_seconds - elapsed))
      if [ "$remaining" -lt 0 ]; then
        remaining=0
      fi
      printf 'wrk... %ss left\n' "$remaining"
    else
      printf 'wrk running...\n'
    fi
    sleep 5
  done
}

parse_wrk_output() {
  read -r RATE_RPS STATUS_FAILURES GRAPHQL_ERRORS RESPONSE_STRUCTURE_FAILURES LATENCY_P50_US LATENCY_P90_US LATENCY_P99_US <<EOF
$(awk '
  /^Requests\/sec:/ { rate=$2 }
  /^VALIDATION_STATUS_FAILURES=/ { split($0, a, "="); status=a[2] }
  /^VALIDATION_GRAPHQL_ERRORS=/ { split($0, a, "="); gql=a[2] }
  /^VALIDATION_RESPONSE_STRUCTURE_FAILURES=/ { split($0, a, "="); structure=a[2] }
  /^LATENCY_P50_US=/ { split($0, a, "="); p50=a[2] }
  /^LATENCY_P90_US=/ { split($0, a, "="); p90=a[2] }
  /^LATENCY_P99_US=/ { split($0, a, "="); p99=a[2] }
  END {
    if (status == "") status = 0
    if (gql == "") gql = 0
    if (structure == "") structure = 0
    if (p50 == "") p50 = "null"
    if (p90 == "") p90 = "null"
    if (p99 == "") p99 = "null"
    print rate, status, gql, structure, p50, p90, p99
  }
' "$WRK_OUTPUT_FILE")
EOF
}

# CPU time used so far by a process, in ms
cpu_ms() {
  if [ -r "/proc/$1/stat" ]; then
    # Linux: utime + stime (fields 14 and 15) are in clock ticks
    awk -v hz="$(getconf CLK_TCK)" '{ print ($14 + $15) * 1000 / hz }' "/proc/$1/stat"
  else
    # macOS: ps prints [hh:]mm:ss.cc
    ps -o time= -p "$1" | awk -F: '{ s = 0; for (i = 1; i <= NF; i++) s = s * 60 + $i; print s * 1000 }'
  fi
}

now_s() {
  # EPOCHREALTIME needs bash 5, macOS /bin/bash falls back to whole seconds
  if [ -n "${EPOCHREALTIME:-}" ]; then echo "$EPOCHREALTIME"; else date +%s; fi
}

# writes "<time s> <router cpu ms>" every second while wrk runs
sample_cpu() {
  while kill -0 "$WRK_PID" 2>/dev/null; do
    echo "$(now_s) $(cpu_ms "$ROUTER_PID")"
    sleep 1
  done > "$CPU_SAMPLES_FILE"
}

# Router CPU per request, p50/p90/p99 over the 1s samples, in µs.
# wrk only reports a total, so each second is divided by the average req/s.
# ponytail: p99 of ~30 samples is basically the max, run longer for a real tail
cpu_per_request_percentiles() {
  if [ -z "${ROUTER_PID:-}" ] || [ ! -s "$CPU_SAMPLES_FILE" ]; then
    echo "null null null"
    return
  fi
  awk -v rps="$RATE_RPS" '
    NR > 1 && $1 > t { v[n++] = ($2 - cpu) * 1000 / (rps * ($1 - t)) }
    { t = $1; cpu = $2 }
    # the first and last second are warm-up and a partial second, skip them
    END { for (i = 1; i < n - 1; i++) print v[i] }
  ' "$CPU_SAMPLES_FILE" | sort -n | awk '
    { v[NR] = $1 }
    function pct(p,  i) { i = int(NR * p / 100 + 0.999999); return v[i < 1 ? 1 : i] }
    END { if (NR == 0) print "null null null"; else print pct(50), pct(90), pct(99) }
  '
}

require_cmd wrk
require_cmd jq

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "$SCRIPT_DIR/.." && pwd)"

ROUTER_ENDPOINT="${ROUTER_ENDPOINT:-http://0.0.0.0:4000/graphql}"
BENCH_CONNECTIONS="${BENCH_CONNECTIONS:-${BENCH_VUS:-50}}"
BENCH_DURATION="${BENCH_DURATION:-${BENCH_OVER_TIME:-30s}}"

if [ -z "${BENCH_THREADS:-}" ]; then
  BENCH_THREADS="$(getconf _NPROCESSORS_ONLN 2>/dev/null || true)"

  if ! [[ "$BENCH_THREADS" =~ ^[0-9]+$ ]] || [ "$BENCH_THREADS" -lt 1 ]; then
    BENCH_THREADS="$(sysctl -n hw.logicalcpu 2>/dev/null || true)"
  fi

  if ! [[ "$BENCH_THREADS" =~ ^[0-9]+$ ]] || [ "$BENCH_THREADS" -lt 1 ]; then
    BENCH_THREADS=1
  fi

  if [ "$BENCH_THREADS" -gt "$BENCH_CONNECTIONS" ]; then
    BENCH_THREADS="$BENCH_CONNECTIONS"
  fi
fi

SUMMARY_PATH="$(resolve_summary_path)"
mkdir -p "$SUMMARY_PATH"
WRK_OUTPUT_FILE="$SUMMARY_PATH/wrk_output.txt"
CPU_SAMPLES_FILE="$SUMMARY_PATH/cpu_samples.txt"

BENCH_OPERATION_FILE="$SCRIPT_DIR/operation.graphql" BENCH_EXPECTED_RESPONSE_FILE="$SCRIPT_DIR/expected_response.json" wrk \
  -t"$BENCH_THREADS" \
  -c"$BENCH_CONNECTIONS" \
  -d"$BENCH_DURATION" \
  --latency \
  -s "$SCRIPT_DIR/wrk.lua" \
  "$ROUTER_ENDPOINT" 2>&1 | tee "$WRK_OUTPUT_FILE" &
WRK_PID=$!

# set ROUTER_PID to also measure the router's CPU per request
SAMPLER_PID=""
if [ -n "${ROUTER_PID:-}" ]; then
  sample_cpu &
  SAMPLER_PID=$!
fi

TOTAL_SECONDS=""
if TOTAL_SECONDS=$(duration_to_seconds "$BENCH_DURATION"); then
  :
fi

show_progress "$WRK_PID" "$TOTAL_SECONDS"
wait "$WRK_PID"
if [ -n "$SAMPLER_PID" ]; then wait "$SAMPLER_PID"; fi

parse_wrk_output

if [ -z "$RATE_RPS" ]; then
  echo "Could not parse Requests/sec from wrk output."
  exit 1
fi

if ! [[ "$STATUS_FAILURES" =~ ^[0-9]+$ ]] || ! [[ "$GRAPHQL_ERRORS" =~ ^[0-9]+$ ]] || ! [[ "$RESPONSE_STRUCTURE_FAILURES" =~ ^[0-9]+$ ]]; then
  echo "Could not parse validation counters from wrk output."
  exit 1
fi

read -r CPU_P50_US CPU_P90_US CPU_P99_US <<EOF
$(cpu_per_request_percentiles)
EOF

VALIDATION_TOTAL_FAILURES=$((STATUS_FAILURES + GRAPHQL_ERRORS + RESPONSE_STRUCTURE_FAILURES))

jq -n \
  --arg tool "wrk" \
  --arg duration "$BENCH_DURATION" \
  --arg endpoint "$ROUTER_ENDPOINT" \
  --arg generated_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  --argjson concurrency "$BENCH_CONNECTIONS" \
  --argjson threads "$BENCH_THREADS" \
  --argjson rate_rps "$RATE_RPS" \
  --argjson status_failures "$STATUS_FAILURES" \
  --argjson graphql_error_responses "$GRAPHQL_ERRORS" \
  --argjson response_structure_failures "$RESPONSE_STRUCTURE_FAILURES" \
  --argjson validation_total_failures "$VALIDATION_TOTAL_FAILURES" \
  --argjson latency_p50_us "$LATENCY_P50_US" \
  --argjson latency_p90_us "$LATENCY_P90_US" \
  --argjson latency_p99_us "$LATENCY_P99_US" \
  --argjson cpu_per_request_p50_us "$CPU_P50_US" \
  --argjson cpu_per_request_p90_us "$CPU_P90_US" \
  --argjson cpu_per_request_p99_us "$CPU_P99_US" \
  '{
    tool: $tool,
    rate_rps: $rate_rps,
    latency_p50_us: $latency_p50_us,
    latency_p90_us: $latency_p90_us,
    latency_p99_us: $latency_p99_us,
    cpu_per_request_p50_us: $cpu_per_request_p50_us,
    cpu_per_request_p90_us: $cpu_per_request_p90_us,
    cpu_per_request_p99_us: $cpu_per_request_p99_us,
    duration: $duration,
    concurrency: $concurrency,
    threads: $threads,
    endpoint: $endpoint,
    generated_at: $generated_at,
    status_failures: $status_failures,
    graphql_error_responses: $graphql_error_responses,
    response_structure_failures: $response_structure_failures,
    validation_total_failures: $validation_total_failures
  }' > "$SUMMARY_PATH/summary.json"

echo "Wrote benchmark summary to $SUMMARY_PATH/summary.json"

if [ "$VALIDATION_TOTAL_FAILURES" -gt 0 ]; then
  echo "Validation failures found in benchmark run."
  exit 1
fi
