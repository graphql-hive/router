## Debug mode:

```
cargo subgraphs

export SUPERGRAPH_FILE_PATH="bench/supergraph.graphql"
cargo router
```

## Release mode:

```
cargo build --release -p subgraphs
cargo build --release -p hive-router

./target/release/subgraphs
./target/release/hive_router bench/supergraph.graphql
```

## Load test

Defaults: concurrency 50 for 30s against `http://0.0.0.0:4000/graphql`

```
# Run
./bench/run-benchmark.sh

# Custom settings
ROUTER_ENDPOINT=http://0.0.0.0:4000/graphql BENCH_CONNECTIONS=69 BENCH_DURATION=10s SUMMARY_PATH=./bench/results/pr ./bench/run-benchmark.sh
```

The runner writes normalized output to `summary.json` in the configured `SUMMARY_PATH`:

```json
{
  "tool": "wrk",
  "rate_rps": 1234.56,
  "duration": "30s",
  "concurrency": 50,
  "endpoint": "http://0.0.0.0:4000/graphql",
  "generated_at": "1700000000",
  "status_failures": 0,
  "graphql_error_responses": 0,
  "validation_total_failures": 0,
  "latency_p95_ms": 12.345,
  "latency_p99_ms": 23.456
}
```

Validation counters are recorded from response inspection (status != 200 and GraphQL payloads containing `"errors"`) but do not fail the run.

## Regression check

Run both benchmarks first (for `./bench/results/pr` and `./bench/results/main`), then:

```
./bench/ci-detect-regression.sh
```

The script compares throughput (`rate_rps`) and fails if PR is more than 5% slower than main.
