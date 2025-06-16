## Profiling using Samply

1. Install `samply` by following: https://github.com/mstange/samply#installation
2. Build the QP dev-cli in profiling mode using: `cargo build --profile profiling -p qp-dev-cli`
3. Run `samply` with your dev-cli args, for example:

```
samply record ./target/profiling/qp-dev-cli plan SUPERGRAPH_PATH OPERATION_PATH
```

## Profiling using Flamegraph

1. Install `flamegraph` by following: https://github.com/flamegraph-rs/flamegraph?tab=readme-ov-file#installation
2. Run `gateway` with the example command.
3. Open the `flamegraph.svg` file

```
cargo flamegraph -p gateway --profile profiling -- SUPERGRAPH_PATH
```
