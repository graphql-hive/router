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

Defaults: 50 vus for 30s

```
cargo run --release -p goose-benchmark

# Custom settings
BENCH_VUS=69 cargo run --release -p goose-benchmark
BENCH_OVER_TIME=10s cargo run --release -p goose-benchmark
```
