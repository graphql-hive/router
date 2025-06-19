## Profiling using Samply


1. Configure `gateway/src/main.rs` to use `#[tokio::main(flavor = "current_thread")]` for better reading of the flamegraph.
2. Install `samply` by following: https://github.com/mstange/samply#installation
3. Build the gateway in profiling mode using: `cargo build --profile profiling -p gateway`
4. Run `samply` with your dev-cli args, for example:

```
samply record ./target/profiling/gateway SUPERGRAPH_PATH
```

## Profiling using Flamegraph

1. Install `flamegraph` by following: https://github.com/flamegraph-rs/flamegraph?tab=readme-ov-file#installation
2. Run `gateway` with the example command.
3. Open the `flamegraph.svg` file

```
cargo flamegraph -p gateway --profile profiling -- SUPERGRAPH_PATH
```
