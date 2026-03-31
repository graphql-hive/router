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

Defaults: 50 connections for 30s

```
./bench/run-benchmark.sh

# Custom settings
BENCH_CONNECTIONS=69 ./bench/run-benchmark.sh
BENCH_DURATION=10s ./bench/run-benchmark.sh

# Backward-compatible env names
BENCH_VUS=69 ./bench/run-benchmark.sh
BENCH_OVER_TIME=10s ./bench/run-benchmark.sh
```
