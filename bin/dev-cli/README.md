## Dev CLI

Use the CLI here to easily get output from the query planner. The following commands are available, for each phase of the plan building. Run them with `cargo dev` (a `cargo run --package qp-dev-cli` alias defined in `.cargo/config.toml`) from anywhere in the workspace:

- `cargo dev consumer_schema SUPERGRAPH_PATH`: constructs and outputs the consumer-facing schema.
- `cargo dev graph SUPERGRAPH_PATH`: constructs and outputs the graph as graphviz.
- `cargo dev paths SUPERGRAPH_PATH OPERATION_PATH`: find best paths for all leafs.
- `cargo dev fetch_graph SUPERGRAPH_PATH OPERATION_PATH`: prints the fetch graph.
- `cargo dev plan SUPERGRAPH_PATH OPERATION_PATH`: plan and print (add `--json` for JSON output).
- `cargo dev normalize SUPERGRAPH_PATH OPERATION_PATH`: prints the normalized operation.
- `cargo dev projection SUPERGRAPH_PATH OPERATION_PATH`: prints the field projection plan.
