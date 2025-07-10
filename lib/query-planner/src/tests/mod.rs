mod alias;
mod arguments;
mod fragments;
mod include_skip;
mod interface;
mod interface_object;
mod interface_object_with_requires;
mod mutations;
mod object_entities;
mod override_requires;
mod overrides;
mod provides;
mod requires;
mod requires_circular;
mod requires_fragments;
mod requires_provides;
mod requires_requires;
mod root_types;
mod testkit;
mod union;

use crate::{
    tests::testkit::{build_query_plan, init_logger},
    utils::parsing::parse_operation,
};

#[test]
fn test_bench_operation() -> Result<(), Box<dyn std::error::Error>> {
    init_logger();
    let document = parse_operation(
        &std::fs::read_to_string("../../bench/operation.graphql")
            .expect("Unable to read input file"),
    );
    let _query_plan = build_query_plan("../../bench/supergraph.graphql", document)?;

    Ok(())
}
