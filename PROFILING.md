## Profiling

1. Install `samply` by following: https://github.com/mstange/samply#installation
2. Build the QP dev-cli in profiling mode using: `cargo build --profile profiling -p qp-dev-cli`
3. Run `samply` with your dev-cli args, for example:

```
samply record ./target/profiling/qp-dev-cli plan SUPERGRAPH_PATH OPERATION_PATH
```
