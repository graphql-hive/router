## Profiling using Flamegraph

1. Install `flamegraph` by following: https://github.com/flamegraph-rs/flamegraph?tab=readme-ov-file#installation
2. Run `router` with the example command.
3. Open the `flamegraph.svg` file

```
cargo flamegraph -p router --profile profiling -- SUPERGRAPH_PATH
```

## Memory profiling

Runs the benchmark query against the benchmark subgraphs with the router built with [dhat](https://docs.rs/dhat) (`--profile memprof --features dhat-heap`, a fast-rebuild profile without LTO), then turns the report into a pprof profile.
Needs `hey`, `jq` and `go`.

```
./bench/memory-profile.sh

# Custom settings
REQUESTS=5000 CONCURRENCY=20 ./bench/memory-profile.sh
ROUTER_CONFIG=bench/configs/no-deduplication.config.yaml ./bench/memory-profile.sh
```

It prints the top allocation sites and leaves `bench/results/memory/heap.pb.gz` behind:

```
go tool pprof -http=:8080 bench/results/memory/heap.pb.gz
```

Sample types (`-sample_index`): `alloc_space`/`alloc_objects` (everything allocated during the run, the default), `peak_space`/`peak_objects` (live at the heap peak), `inuse_space`/`inuse_objects` (still live at exit).

To compare changes, save a baseline once and every next run prints the diff:

```
cp bench/results/memory/heap.pb.gz bench/results/memory/baseline.pb.gz
```
