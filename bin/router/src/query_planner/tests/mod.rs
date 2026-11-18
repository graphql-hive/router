mod alias;
mod arguments;
mod context;
mod dependencies;
mod entity_reuse;
mod fragments;
mod generated;
mod include_skip;
mod interface;
mod interface_object;
mod interface_object_with_requires;
mod issues;
mod mutations;
mod object_entities;
mod override_requires;
mod overrides;
mod provides;
mod renamed_root_types;
mod requires;
mod requires_alias_scopes;
mod requires_circular;
mod requires_fragments;
mod requires_provides;
mod requires_requires;
mod root_reentry;
mod root_types;
pub(crate) mod testkit;
mod union;

use crate::query_planner::{
    tests::testkit::{build_query_plan_with_defaults, init_logger, read_supergraph_state},
    utils::parsing::parse_operation,
};

#[test]
fn test_bench_operation() -> Result<(), Box<dyn std::error::Error>> {
    init_logger();
    let document = parse_operation(
        &std::fs::read_to_string("../../bench/operation.graphql")
            .expect("Unable to read input file"),
    );
    let query_plan = build_query_plan_with_defaults("../../bench/supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [15] products       -
      [2] accounts        -
      [17] inventory      [15]
      [20] reviews        [15], [2]
      [25] products       [20]
      [21] accounts       [20]
      [22] inventory      [25]

    [15] products
      out
        { topProducts { __typename upc name price weight } }

    [2] accounts
      out
        { users { __typename id username name } }

    [17] inventory   after [15]
      merge
        topProducts.@
      in
        ... on Product { __typename price weight upc }
      out
        ... on Product { shippingEstimate inStock }

    [20] reviews   after [15], [2]
      alias _e0
        merge
          topProducts.@
        in
          ... on Product { __typename upc }
        out
          ... on Product { reviews { ...a } }
      alias _e1
        merge
          users.@
        in
          ... on User { __typename id }
        out
          ... on User { reviews { id body product { __typename upc reviews { ...a } } } }
      fragments
        fragment a on Review {
          id
          body
          author {
            __typename
            id
            username
            reviews { id body product { __typename upc } }
          }
        }

    [25] products   after [20]
      alias _e0
        merge
          topProducts.@.reviews.@.author.reviews.@.product
          users.@.reviews.@.product
          users.@.reviews.@.product.reviews.@.author.reviews.@.product
        in
          ... on Product { __typename upc }
        out
          ... on Product { price weight name }

    [21] accounts   after [20]
      alias _e0
        merge
          topProducts.@.reviews.@.author
          users.@.reviews.@.product.reviews.@.author
        in
          ... on User { __typename id }
        out
          ... on User { name }

    [22] inventory   after [25]
      alias _e0
        merge
          topProducts.@.reviews.@.author.reviews.@.product
          users.@.reviews.@.product
          users.@.reviews.@.product.reviews.@.author.reviews.@.product
        in
          ... on Product { __typename upc price weight }
        out
          ... on Product { inStock shippingEstimate }
    ");

    Ok(())
}

/// Converts the benchmark plan to its executable form. The serialized output is what other tools
/// depend on, so it must stay the same after the planning data is dropped.
#[test]
fn preparing_a_plan_preserves_its_wire_form() -> Result<(), Box<dyn std::error::Error>> {
    init_logger();
    let document = parse_operation(&std::fs::read_to_string("../../bench/operation.graphql")?);
    let planning = build_query_plan_with_defaults("../../bench/supergraph.graphql", document)?;
    let before = serde_json::to_string(&planning)?;
    let executable =
        planning.into_executable(&read_supergraph_state("../../bench/supergraph.graphql"));
    assert_eq!(serde_json::to_string(&executable)?, before);
    Ok(())
}
