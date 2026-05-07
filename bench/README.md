## Debug mode:

```
cargo subgraphs

export SUPERGRAPH_FILE_PATH="bench/supergraph.graphql"
cargo router
```

## Monolithic baseline

`bench/monolith-js` is the monolithic baseline used for differential testing.

It runs a small GraphQL Yoga monolith over the same benchmark domain data as the federated benchmark stack, so `graphql-diff` can compare monolithic and federated execution results with a shared domain model.

```bash
cd bench/monolith-js
npm install
npm start
```

By default it serves GraphQL at `http://localhost:4300/graphql`.

## Differential testing

With `bench/monolith-js` running as the baseline and the router running in front of `bench/subgraphs`, you can run differential testing with:

```bash
cargo run -p graphql-differential -- \
  http://localhost:4300/graphql \
  http://localhost:4000/graphql \
  bench/schema.graphql
```

When results differ, the generated query, variables, and both endpoint responses are written to `failed-tests/`.

## Release mode:

```
cargo build --release -p subgraphs
cargo build --release -p hive-router

./target/release/subgraphs
./target/release/hive_router bench/supergraph.graphql
```

## Coprocessor benchmark server (h2c over UDS)

```
cargo build -p bench-coprocessor --release
./target/release/bench_coprocessor
```

## Load test

Defaults: 50 connections for 30s

```
./bench/run-benchmark.sh

# Custom settings
BENCH_CONNECTIONS=69 ./bench/run-benchmark.sh
BENCH_DURATION=10s ./bench/run-benchmark.sh
BENCH_PERSISTED_MODE=true BENCH_DOCUMENT_ID=bench_test_query ./bench/run-benchmark.sh

# Backward-compatible env names
BENCH_VUS=69 ./bench/run-benchmark.sh
BENCH_OVER_TIME=10s ./bench/run-benchmark.sh
```
