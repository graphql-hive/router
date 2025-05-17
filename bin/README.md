## Dev CLI

Use the CLI here to easily get output from the query planner. The following commands are available, for each phase of the plan building (run from the root of the workspace):

- `cargo run graph SUPERGRAPH_PATH`: constructs and outputs the graph as graphviz.
- `cargo run paths SUPERGRAPH_PATH OPERATION_PATH`: find best paths for all leafs.
- `cargo run trees SUPERGRAPH_PATH OPERATION_PATH`: find best paths for all leafs, and prints the query tree for each path found.
- `cargo run merged_tree SUPERGRAPH_PATH OPERATION_PATH`: find best paths for all leafs, and prints the merged fetch tree for all fields.
- `cargo run fetch_graph SUPERGRAPH_PATH OPERATION_PATH`: prints the fetch graph
