# Hive-Router's Query-Planner (`hive-router-query-planner`)

This crate is a standalone library for performing GraphQL query-planning for Federation >=v2.

## Installation

Add this to your Cargo.toml:

```toml
[dependencies]
hive-router-query-planner = "<...>"
```

## Usage

```rs
use hive_router_query_planner::ast::normalization::normalize_operation;
use hive_router_query_planner::planner::Planner;
use hive_router_query_planner::utils::parsing::{parse_schema, safe_parse_operation};

fn main() {
  // First, parse your Federation supergraph SDL
  let parsed_supergraph = parse_schema("...");
  // Next, create a Planner object (only once)
  let planner = Planner::new_from_supergraph(&parsed_supergraph).expect("failed to create planner");

  // For every operation you wish to plan, parse the operation
  let parsed_operation = safe_parse_operation("...").expect("failed to parse operation");
  let operation_name: Option<String> = None; // Set this from your input, or keep empty
  // Normalize the operation
  let normalized_operation = normalize_operation(
      &planner.supergraph,
      &parsed_operation,
      operation_name.as_deref(),
  );

  // And then create a query plan
  let plan = planner.plan_from_normalized_operation(&normalized_operation.operation, Default::default()).expect("failed to plan");

  // Print the plan
  println!("{:?}", plan);
  // Or serialize it to JSON
  let json = serde_json::to_string(&plan).expect("failed to serialize plan");
  println!("{}", json);
}
```

Once a plan is produced, it can be executed and using [the `hive-router-plan-executor` crate](../executor).

## Local Development

### Testing

To run all tests for the QP, please use:

```
cargo test_qp
```

To run a specific test, or a specific test suite, please use:

```
cargo test_qp file_or_fn_name
# OR
cargo test_qp tests::file_name::test_name
```

### Logging

* To see log messages created by `println!` please also pass `--nocapture`

```
cargo test_qp --nocapture
```

* To see logs created by the QP itself, using `tracing` or `instrumente` macro, please set `RUST_LOG="..."` (see [EnvFilter](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html#example-syntax)):

```
RUST_LOG="debug" cargo test_qp
```

### Snapshots

We are using `insta` for snapshots. To get started, make sure to install [`insta` cli command on Cargo](https://insta.rs/docs/cli/).

Create a test and snapshot it by using `@""` for inline or `""` for standalone snapshot file. Run the test and allow it to fail due to invalid/missing snapshot.

To review snapshots, run `cargo insta review`.
