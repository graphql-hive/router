## Profiling using Flamegraph

1. Install `flamegraph` by following: https://github.com/flamegraph-rs/flamegraph?tab=readme-ov-file#installation
2. Run `gateway` with the example command.
3. Open the `flamegraph.svg` file

```
cargo flamegraph -p gateway --profile profiling -- SUPERGRAPH_PATH
```
