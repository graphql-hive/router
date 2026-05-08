use crate::{
    tests::testkit::{build_query_plan, init_logger},
    utils::parsing::parse_operation,
};
use std::error::Error;

/// Regression test: multiple inline fragments on the same concrete type inside an abstract type
/// fragment should all be evaluated, not just the first one.
/// Uses `account(id:)` (concrete parent type `Account`) with a `... on Node` abstract fragment
/// to force the `expand_abstract_fragment` path, where two `... on Account` inline fragments
/// must both be collected and merged so all their fields appear in the query plan.
#[test]
fn multiple_inline_fragments_on_same_concrete_type_within_interface_fragment(
) -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          account(id: "a1") {
            ... on Node {
              ... on Account {
                id
              }
              ... on Account {
                username
              }
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan(
        "fixture/tests/corrupted-supergraph-node-id.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Fetch(service: "a") {
        {
          account(id: "a1") {
            id
            username
          }
        }
      },
    },
    "#);

    Ok(())
}

/// Regression test: when an abstract-type fragment carries a directive (e.g. `@include`)
/// and a nested concrete-type fragment carries another `@include` with a *different*
/// argument, both conditions must be preserved in the query plan. Because `@include`/
/// `@skip` are non-repeatable, the parent directive must be carried by an outer wrapper
/// fragment around the inner one.
#[test]
fn nested_same_name_directives_on_abstract_and_concrete_fragments() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query ($parent: Boolean!, $child: Boolean!) {
          account(id: "a1") {
            ... on Node @include(if: $parent) {
              ... on Account @include(if: $child) {
                username
              }
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan(
        "fixture/tests/corrupted-supergraph-node-id.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Fetch(service: "a") {
        query ($child:Boolean!,$parent:Boolean!) {
          account(id: "a1") {
            ... on Account @include(if: $parent) {
              ... on Account @include(if: $child) {
                username
              }
            }
          }
        }
      },
    },
    "#);

    Ok(())
}

/// When the parent abstract fragment and the nested concrete fragment carry the
/// *same* directive with the *same* argument, the duplicate must be collapsed —
/// no redundant nesting should appear in the plan.
#[test]
fn nested_same_directive_same_arg_on_abstract_and_concrete_fragments() -> Result<(), Box<dyn Error>>
{
    init_logger();
    let document = parse_operation(
        r#"
        query ($cond: Boolean!) {
          account(id: "a1") {
            ... on Node @include(if: $cond) {
              ... on Account @include(if: $cond) {
                username
              }
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan(
        "fixture/tests/corrupted-supergraph-node-id.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Fetch(service: "a") {
        query ($cond:Boolean!) {
          account(id: "a1") {
            ... on Account @include(if: $cond) {
              username
            }
          }
        }
      },
    },
    "#);

    Ok(())
}

/// When the parent abstract fragment carries `@skip` and the nested concrete fragment
/// carries `@include`, both directives must be preserved (different names, so they can
/// be merged onto the same fragment).
#[test]
fn nested_different_directives_on_abstract_and_concrete_fragments() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query ($skip: Boolean!, $include: Boolean!) {
          account(id: "a1") {
            ... on Node @skip(if: $skip) {
              ... on Account @include(if: $include) {
                username
              }
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan(
        "fixture/tests/corrupted-supergraph-node-id.supergraph.graphql",
        document,
    )?;

    let plan_str = format!("{}", query_plan);
    assert!(
        plan_str.contains("$skip"),
        "parent @skip directive must be preserved: {plan_str}"
    );
    assert!(
        plan_str.contains("$include"),
        "child @include directive must be preserved: {plan_str}"
    );

    insta::assert_snapshot!(plan_str, @r#"
    QueryPlan {
      Fetch(service: "a") {
        query ($include:Boolean!,$skip:Boolean!) {
          account(id: "a1") {
            ... on Account @skip(if: $skip) @include(if: $include) {
              username
            }
          }
        }
      },
    },
    "#);

    Ok(())
}

/// When the parent abstract fragment carries `@include` and the nested concrete fragment
/// has *no* directive, the parent's `@include` must be merged onto the concrete fragment.
#[test]
fn parent_directive_only_on_abstract_fragment() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query ($parent: Boolean!) {
          account(id: "a1") {
            ... on Node @include(if: $parent) {
              ... on Account {
                username
              }
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan(
        "fixture/tests/corrupted-supergraph-node-id.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Fetch(service: "a") {
        query ($parent:Boolean!) {
          account(id: "a1") {
            ... on Account @include(if: $parent) {
              username
            }
          }
        }
      },
    },
    "#);

    Ok(())
}

/// Reusing the same named fragment with different `@include` conditions on the same
/// concrete parent must preserve both conditions as separate wrappers.
#[test]
fn reusable_fragment_with_mixed_include_conditions_on_concrete_parent() -> Result<(), Box<dyn Error>>
{
    init_logger();
    let document = parse_operation(
        r#"
        query ($first: Boolean!, $second: Boolean!) {
          account(id: "a1") {
            ...AccountFields @include(if: $first)
            ...AccountFields @include(if: $second)
          }
        }

        fragment AccountFields on Account {
          username
        }
        "#,
    );
    let query_plan = build_query_plan(
        "fixture/tests/corrupted-supergraph-node-id.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Fetch(service: "a") {
        query ($first:Boolean!,$second:Boolean!) {
          account(id: "a1") {
            ... on Account @include(if: $first) {
              ...a
            }
            ... on Account @include(if: $second) {
              ...a
            }
          }
        }
        fragment a on Account {
          username
        }
      },
    },
    "#);

    Ok(())
}

/// Reusing the same named fragment with different `@include` conditions under an
/// abstract parent must keep both conditions through type expansion.
#[test]
fn reusable_fragment_with_mixed_include_conditions_on_abstract_parent() -> Result<(), Box<dyn Error>>
{
    init_logger();
    let document = parse_operation(
        r#"
        query ($first: Boolean!, $second: Boolean!) {
          account(id: "a1") {
            ... on Node {
              ...AccountFields @include(if: $first)
              ...AccountFields @include(if: $second)
            }
          }
        }

        fragment AccountFields on Account {
          username
        }
        "#,
    );
    let query_plan = build_query_plan(
        "fixture/tests/corrupted-supergraph-node-id.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Fetch(service: "a") {
        query ($first:Boolean!,$second:Boolean!) {
          account(id: "a1") {
            ... on Account @include(if: $first) {
              ...a
            }
            ... on Account @include(if: $second) {
              ...a
            }
          }
        }
        fragment a on Account {
          username
        }
      },
    },
    "#);

    Ok(())
}

#[test]
fn simple_inline_fragment() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
            query {
              products {
                price {
                  amount
                  currency
                }
                ... on Product {
                  isAvailable
                }
              }
            }"#,
    );
    let query_plan = build_query_plan("fixture/tests/testing.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "store") {
          {
            products {
              __typename
              id
            }
          }
        },
        Flatten(path: "products") {
          Fetch(service: "info") {
            {
              ... on Product {
                __typename
                id
              }
            } =>
            {
              ... on Product {
                isAvailable
                uuid
              }
            }
          },
        },
        Flatten(path: "products") {
          Fetch(service: "cost") {
            {
              ... on Product {
                __typename
                uuid
              }
            } =>
            {
              ... on Product {
                price {
                  amount
                  currency
                }
              }
            }
          },
        },
      },
    },
    "#);

    Ok(())
}

#[test]
fn fragment_spread() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        fragment ProductInfo on Product {
          isAvailable
        }

        query {
          products {
            price {
              amount
              currency
            }
            ...ProductInfo
          }
        }"#,
    );
    let query_plan = build_query_plan("fixture/tests/testing.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "store") {
          {
            products {
              __typename
              id
            }
          }
        },
        Flatten(path: "products") {
          Fetch(service: "info") {
            {
              ... on Product {
                __typename
                id
              }
            } =>
            {
              ... on Product {
                isAvailable
                uuid
              }
            }
          },
        },
        Flatten(path: "products") {
          Fetch(service: "cost") {
            {
              ... on Product {
                __typename
                uuid
              }
            } =>
            {
              ... on Product {
                price {
                  amount
                  currency
                }
              }
            }
          },
        },
      },
    },
    "#);

    Ok(())
}
