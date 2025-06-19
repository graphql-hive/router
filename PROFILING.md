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

## Profiling with perfetto

1. Build GW in release mode: `cargo build --release -p gateway`
2. Run gateway in release mode with the following flag: `PERFETTO_OUT="1" RUST_LOG="trace" ./target/release/gateway bench/supergraph.graphql`
3. Use the generated `trace-*.json` file and load it into https://ui.perfetto.dev
