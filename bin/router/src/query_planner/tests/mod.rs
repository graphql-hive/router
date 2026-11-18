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
mod plan_oracle;
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
      [13] products       -
      [2] accounts        -
      [15] inventory      [13]
      [17] reviews        [13], [2]
      [22] products       [17]
      [18] accounts       [17]
      [19] inventory      [22]

    [13] products
      out
        { topProducts { __typename upc name price weight } }

    [2] accounts
      out
        { users { __typename id username name } }

    [15] inventory   after [13]
      merge
        topProducts.@
      in
        ... on Product { __typename price weight upc }
      out
        ... on Product { shippingEstimate inStock }

    [17] reviews   after [13], [2]
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

    [22] products   after [17]
      alias _e0
        merge
          topProducts.@.reviews.@.author.reviews.@.product
          users.@.reviews.@.product.reviews.@.author.reviews.@.product
          users.@.reviews.@.product
        in
          ... on Product { __typename upc }
        out
          ... on Product { name price weight }

    [18] accounts   after [17]
      alias _e0
        merge
          topProducts.@.reviews.@.author
          users.@.reviews.@.product.reviews.@.author
        in
          ... on User { __typename id }
        out
          ... on User { name }

    [19] inventory   after [22]
      alias _e0
        merge
          topProducts.@.reviews.@.author.reviews.@.product
          users.@.reviews.@.product.reviews.@.author.reviews.@.product
          users.@.reviews.@.product
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
