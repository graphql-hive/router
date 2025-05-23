# Query Planner

## Usage

```rs
fn main() {
  let parsed_supergraph = parse_schema("...");
  let planner = Planner::new_from_supergraph(&parsed_supergraph);
  let operation = parse_operation("...");
  let plan = planner.plan(&operation);
}
```

## Testing

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

## Logging

* To see log messages created by `println!` please also pass `--nocapture`

```
cargo test_qp --nocapture
```

* To see logs created by the QP itself, using `tracing` or `instrumente` macro, please set `DEBUG=1`

```
DEBUG=1 cargo test_qp
```

## Snapshots

We are using `insta` for snapshots. To get started, make sure to install [`insta` cli command on Cargo](https://insta.rs/docs/cli/).

Create a test and snapshot it by using `@""` for inline or `""` for standalone snapshot file. Run the test and allow it to fail due to invalid/missing snapshot.

To review snapshots, run `cargo insta review`.
