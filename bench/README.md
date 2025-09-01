## Debug mode:

```
cargo subgraphs

export HIVE_SUPERGRAPH_SOURCE="file"
export HIVE_SUPERGRAPH_PATH="bench/supergraph.graphql"
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
k6 run k6.js

# Custom settings
k6 run k6.js -e BENCH_VUS=69
k6 run k6.js -e BENCH_OVER_TIME=10s
```
