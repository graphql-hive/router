use crate::query_planner::{
    tests::testkit::{build_query_plan_with_defaults, init_logger},
    utils::parsing::parse_operation,
};
use std::error::Error;

// TODO: add a test that involves an entity call to fetch non-local fields from Book and Movie
//       to test how `... on X` affects the FetchGraph and FlattenNode.
#[test]
fn union_member_resolvable() -> Result<(), Box<dyn Error>> {
    init_logger();

    let document = parse_operation(
        r#"
        query {
          media {
            ... on Book {
              title
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/union-intersection.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Fetch(service: "a") {
        {
          media {
            __typename
            ... on Book {
              title
            }
          }
        }
      },
    },
    "#);

    Ok(())
}

#[test]
fn union_member_unresolvable() -> Result<(), Box<dyn Error>> {
    init_logger();

    let document = parse_operation(
        r#"
        query {
          media {
            ... on Movie {
              title
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/union-intersection.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Fetch(service: "a") {
        {
          media {
            __typename
          }
        }
      },
    },
    "#);

    Ok(())
}

#[test]
fn union_member_mix() -> Result<(), Box<dyn Error>> {
    init_logger();

    let document = parse_operation(
        r#"
        query {
          media {
            __typename
            ... on Book {
              title
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/union-intersection.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Fetch(service: "a") {
        {
          media {
            __typename
            ... on Book {
              title
            }
          }
        }
      },
    },
    "#);

    Ok(())
}

#[test]
fn union_member_entity_call() -> Result<(), Box<dyn Error>> {
    init_logger();

    let document = parse_operation(
        r#"
        query {
          aMedia {
            __typename
            ... on Book {
              title
              aTitle
              bTitle
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/union-intersection.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          {
            aMedia {
              __typename
              ... on Book {
                __typename
                title
                aTitle
                id
              }
            }
          }
        },
        Flatten(path: "aMedia|[Book]") {
          Fetch(service: "b") {
            {
              ... on Book {
                __typename
                id
              }
            } =>
            {
              ... on Book {
                bTitle
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
fn union_member_entity_call_many_local() -> Result<(), Box<dyn Error>> {
    init_logger();

    let document = parse_operation(
        r#"
        query {
          viewer {
            song {
              __typename
              ... on Song {
                title
                aTitle
              }
              ... on Movie {
                title
                bTitle
              }
              ... on Book {
                title
                aTitle
                bTitle
              }
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/union-intersection.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          {
            viewer {
              song {
                __typename
                ... on Song {
                  title
                  aTitle
                }
                ... on Book {
                  __typename
                  title
                  aTitle
                  id
                }
              }
            }
          }
        },
        Flatten(path: "viewer.song|[Book]") {
          Fetch(service: "b") {
            {
              ... on Book {
                __typename
                id
              }
            } =>
            {
              ... on Book {
                bTitle
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
fn union_member_entity_call_many() -> Result<(), Box<dyn Error>> {
    init_logger();

    let document = parse_operation(
        r#"
        query {
          viewer {
            media {
              __typename
              ... on Song {
                title
                aTitle
              }
              ... on Movie {
                title
                bTitle
              }
              ... on Book {
                title
                aTitle
                bTitle
              }
            }
            book {
              __typename
              ... on Song {
                title
                aTitle
              }
              ... on Movie {
                title
                bTitle
              }
              ... on Book {
                title
                aTitle
                bTitle
              }
            }
            song {
              __typename
              ... on Song {
                title
                aTitle
              }
              ... on Movie {
                title
                bTitle
              }
              ... on Book {
                title
                aTitle
                bTitle
              }
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/union-intersection.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Parallel {
          Fetch(service: "b") {
            {
              viewer {
                media {
                  ...a
                }
                book {
                  ...a
                }
              }
            }
            fragment a on ViewerMedia {
              __typename
              ... on Book {
                bTitle
              }
            }
          },
          Fetch(service: "a") {
            {
              viewer {
                media {
                  ...a
                }
                book {
                  ...a
                }
                song {
                  __typename
                  ... on Song {
                    title
                    aTitle
                  }
                  ... on Book {
                    __typename
                    title
                    aTitle
                    id
                  }
                }
              }
            }
            fragment a on ViewerMedia {
              __typename
              ... on Book {
                aTitle
                title
              }
            }
          },
        },
        Flatten(path: "viewer.song|[Book]") {
          Fetch(service: "b") {
            {
              ... on Book {
                __typename
                id
              }
            } =>
            {
              ... on Book {
                bTitle
              }
            }
          },
        },
      },
    },
    "#);

    Ok(())
}

// Related: https://github.com/graphql-hive/router/issues/1098
// Related: https://github.com/graphql-hive/federation-gateway-audit/pull/347
#[test]
fn partial_union_member_only_in_one_subgraph() -> Result<(), Box<dyn Error>> {
    init_logger();

    let document = parse_operation(
        r#"
        query {
          getResponse {
            message
            actions {
              __typename
              ... on Alpha {
                id
                value
              }
              ... on Beta {
                id
                name
                details
              }
              ... on Gamma {
                id
                label
              }
            }
          }
        }
        "#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/tests/partial-union.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Fetch(service: "a") {
        {
          getResponse {
            message
            actions {
              __typename
              ... on Alpha {
                id
                value
              }
              ... on Beta {
                id
                name
                details
              }
              ... on Gamma {
                id
                label
              }
            }
          }
        }
      },
    },
    "#);

    Ok(())
}

// Related: https://github.com/graphql-hive/federation-gateway-audit/pull/347
#[test]
fn partial_union_members_missing_from_other_subgraph_only() -> Result<(), Box<dyn Error>> {
    init_logger();

    let document = parse_operation(
        r#"
        query {
          getResponse {
            actions {
              __typename
              ... on Beta {
                name
              }
              ... on Gamma {
                label
              }
            }
          }
        }
        "#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/tests/partial-union.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Fetch(service: "a") {
        {
          getResponse {
            actions {
              __typename
              ... on Beta {
                name
              }
              ... on Gamma {
                label
              }
            }
          }
        }
      },
    },
    "#);

    Ok(())
}

#[test]
fn nested_partial_union_entity_representable_a_path_keeps_only_common_members(
) -> Result<(), Box<dyn Error>> {
    init_logger();

    let document = parse_operation(
        r#"
        query {
          rootA {
            wrapper {
              actions {
                __typename
                ... on Common {
                  label
                }
                ... on OnlyA {
                  a
                }
                ... on OnlyB {
                  b
                }
              }
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/partial-union-complex.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Fetch(service: "a") {
        {
          rootA {
            wrapper {
              actions {
                __typename
                ... on Common {
                  label
                }
              }
            }
          }
        }
      },
    },
    "#);

    Ok(())
}

#[test]
fn nested_partial_union_entity_representable_b_path_keeps_only_common_members(
) -> Result<(), Box<dyn Error>> {
    init_logger();

    let document = parse_operation(
        r#"
        query {
          rootB {
            wrapper {
              actions {
                __typename
                ... on Common {
                  label
                }
                ... on OnlyA {
                  a
                }
                ... on OnlyB {
                  b
                }
              }
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/partial-union-complex.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Fetch(service: "b") {
        {
          rootB {
            wrapper {
              actions {
                __typename
                ... on Common {
                  label
                }
              }
            }
          }
        }
      },
    },
    "#);

    Ok(())
}

#[test]
fn nested_partial_union_missing_member_from_pinned_path_falls_back_to_typename(
) -> Result<(), Box<dyn Error>> {
    init_logger();

    let document = parse_operation(
        r#"
        query {
          rootA {
            wrapper {
              actions {
                __typename
                ... on OnlyB {
                  b
                }
              }
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/partial-union-complex.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Fetch(service: "a") {
        {
          rootA {
            wrapper {
              actions {
                __typename
              }
            }
          }
        }
      },
    },
    "#);

    Ok(())
}

#[test]
fn nested_partial_union_ambiguous_shareable_path_keeps_only_common_members(
) -> Result<(), Box<dyn Error>> {
    init_logger();

    let document = parse_operation(
        r#"
        query {
          shared {
            wrapper {
              actions {
                __typename
                ... on Common {
                  label
                }
                ... on OnlyA {
                  a
                }
                ... on OnlyB {
                  b
                }
              }
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/partial-union-complex.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Fetch(service: "a") {
        {
          shared {
            wrapper {
              actions {
                __typename
                ... on Common {
                  label
                }
              }
            }
          }
        }
      },
    },
    "#);

    Ok(())
}

#[test]
fn nested_partial_union_uses_members_from_subgraph_reached_by_entity_move(
) -> Result<(), Box<dyn Error>> {
    init_logger();

    let document = parse_operation(
        r#"
        query {
          rootA {
            bWrapper {
              actions {
                __typename
                ... on Common {
                  label
                }
                ... on OnlyA {
                  a
                }
                ... on OnlyB {
                  b
                }
              }
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/partial-union-complex.supergraph.graphql",
        document,
    )?;
    let query_plan = format!("{}", query_plan);

    // bWrapper forces an entity move from Container/a to Container/b before resolving actions,
    // so Action must be narrowed to B's local members and keep OnlyB while dropping OnlyA.
    assert!(query_plan.contains("... on OnlyB"));
    assert!(!query_plan.contains("... on OnlyA"));

    insta::assert_snapshot!(query_plan, @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          {
            rootA {
              __typename
              id
            }
          }
        },
        Flatten(path: "rootA") {
          Fetch(service: "b") {
            {
              ... on Container {
                __typename
                id
              }
            } =>
            {
              ... on Container {
                bWrapper {
                  actions {
                    __typename
                    ... on Common {
                      label
                    }
                    ... on OnlyB {
                      b
                    }
                  }
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
fn union_overfetching_test() -> Result<(), Box<dyn Error>> {
    init_logger();

    let document = parse_operation(
        r#"
        query {
          review {
            ... on AnonymousReview {
              product {
                b
              }
            }
            ... on UserReview {
              product {
                c
                d
              }
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/union-overfetching.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          {
            review {
              __typename
              ... on AnonymousReview {
                product {
                  ...a
                }
              }
              ... on UserReview {
                product {
                  ...a
                }
              }
            }
          }
          fragment a on Product {
            __typename
            id
          }
        },
        Parallel {
          Flatten(path: "review|[UserReview].product") {
            Fetch(service: "d") {
              {
                ... on Product {
                  __typename
                  id
                }
              } =>
              {
                ... on Product {
                  d
                }
              }
            },
          },
          Flatten(path: "review|[UserReview].product") {
            Fetch(service: "c") {
              {
                ... on Product {
                  __typename
                  id
                }
              } =>
              {
                ... on Product {
                  c
                }
              }
            },
          },
          Flatten(path: "review|[AnonymousReview].product") {
            Fetch(service: "b") {
              {
                ... on Product {
                  __typename
                  id
                }
              } =>
              {
                ... on Product {
                  b
                }
              }
            },
          },
        },
      },
    },
    "#);

    Ok(())
}

#[test]
fn union_list_test() -> Result<(), Box<dyn Error>> {
    init_logger();

    let document = parse_operation(
        r#"
        {
          orders {
            ... on OrdersPageConnections {
              items {
                id
                items {
                  ... on PaperBook {
                    product {
                      id
                      name
                      slug
                    }
                  }
                  ... on DigitalBook {
                    product {
                      id
                      name
                      slug
                    }
                  }
                }
              }
            }
          }
        }
        "#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/tests/union-list.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          {
            orders {
              __typename
              ... on OrdersPageConnections {
                items {
                  id
                  items {
                    __typename
                    ... on PaperBook {
                      product {
                        ...a
                      }
                    }
                    ... on DigitalBook {
                      product {
                        ...a
                      }
                    }
                  }
                }
              }
            }
          }
          fragment a on Product {
            __typename
            id
          }
        },
        Flatten(path: "orders|[OrdersPageConnections].items.@.items.@.product") {
          Fetch(service: "b") {
            {
              ... on Product {
                __typename
                id
              }
            } =>
            {
              ... on Product {
                name
                slug
              }
            }
          },
        },
      },
    },
    "#);

    Ok(())
}

/// https://github.com/graphql-hive/router/issues/1732
///
/// Each union member wants other fields of its `Product`. One entity call for all three would
/// ask every product for `a`, `b` and `c`, whatever member it's under. Each member gets its own
/// alias instead, still in one request.
#[test]
fn union_members_ask_only_for_their_own_entity_fields() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        {
          card {
            components {
              __typename
              ... on ComponentA { product { id a } }
              ... on ComponentB { product { id b } }
              ... on ComponentC { product { id c } }
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/union-member-entity-fields.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "card") {
          {
            card {
              components {
                __typename
                ... on ComponentA {
                  product {
                    ...a
                  }
                }
                ... on ComponentB {
                  product {
                    ...a
                  }
                }
                ... on ComponentC {
                  product {
                    ...a
                  }
                }
              }
            }
          }
          fragment a on Product {
            __typename
            id
          }
        },
        BatchFetch(service: "product") {
          {
            _e0 {
              paths: [
                "card.components.@|[ComponentC].product"
              ]
              {
                ... on Product {
                  __typename
                  id
                }
              }
            }
            _e1 {
              paths: [
                "card.components.@|[ComponentB].product"
              ]
              {
                ... on Product {
                  __typename
                  id
                }
              }
            }
            _e2 {
              paths: [
                "card.components.@|[ComponentA].product"
              ]
              {
                ... on Product {
                  __typename
                  id
                }
              }
            }
          }
          {
            _e0: _entities(representations: $__batch_reps_0) {
              ... on Product {
                c
              }
            }
            _e1: _entities(representations: $__batch_reps_1) {
              ... on Product {
                b
              }
            }
            _e2: _entities(representations: $__batch_reps_2) {
              ... on Product {
                a
              }
            }
          }
        },
      },
    },
    "#);

    Ok(())
}
