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
                    ... on DigitalBook {
                      product {
                        ...a
                      }
                    }
                    ... on PaperBook {
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

/// `components` points at one `Component` node per member. A conditional `... on Component`
/// stays on each of them, with the condition, and every member still asks only for its own
/// entity field.
#[test]
fn conditional_union_type_condition_on_its_own_field_include_mixed_members(
) -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query($v: Boolean!) {
          card {
            components {
              ... on Component @include(if: $v) {
                __typename
                ... on ComponentA { product { id a } }
                ... on ComponentC { product { id c } }
              }
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
          query ($v:Boolean!) {
            card {
              components {
                ... on Component @include(if: $v) {
                  __typename
                  ... on ComponentA {
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
          }
          fragment a on Product {
            __typename
            id
          }
        },
        BatchFetch(service: "product") {
          {
            _e0(if: $v) {
              paths: [
                "card.components.@|[Component]|[ComponentC].product"
              ]
              {
                ... on Product {
                  __typename
                  id
                }
              }
            }
            _e1(if: $v) {
              paths: [
                "card.components.@|[Component]|[ComponentA].product"
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

/// The same with the members asking for the same fields, so `ComponentA` and `ComponentC` are
/// planned once, for `ComponentA` with `ComponentC` too. The wrapper's selfie edge comes after
/// the `components` field, and every member still gets its `product`.
#[test]
fn conditional_union_type_condition_on_its_own_field_include_grouped_members(
) -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query($v: Boolean!) {
          card {
            components {
              ... on Component @include(if: $v) {
                __typename
                ... on ComponentA { product { id a } }
                ... on ComponentC { product { id a } }
              }
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
          query ($v:Boolean!) {
            card {
              components {
                ... on Component @include(if: $v) {
                  __typename
                  ... on ComponentA {
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
          }
          fragment a on Product {
            __typename
            id
          }
        },
        Flatten(path: "card.components.@|[Component]|[ComponentA|ComponentC].product") {
          Fetch(service: "product", if: $v) {
            {
              ... on Product {
                __typename
                id
              }
            } =>
            {
              ... on Product {
                a
              }
            }
          },
        },
      },
    },
    "#);

    Ok(())
}

/// The review's operation: conditions on the union and on a member inside it.
#[test]
fn conditional_member_inside_a_conditional_union_type_condition() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query($a: Boolean!, $b: Boolean!) {
          card {
            components {
              ... on Component @skip(if: $a) {
                ... on ComponentC @skip(if: $b) {
                  product { b }
                }
              }
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
          query ($a:Boolean!,$b:Boolean!) {
            card {
              components {
                ... on Component @skip(if: $a) {
                  __typename
                  ... on ComponentC {
                    product @skip(if: $b) {
                      __typename
                      id
                    }
                  }
                }
              }
            }
          }
        },
        Flatten(path: "card.components.@|[Component]|[ComponentC].product") {
          Fetch(service: "product", if: !$a && !$b) {
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
    "#);

    Ok(())
}

/// `a`'s `actions` has only `Common`, the condition doesn't let `OnlyA` or `OnlyB` back in.
#[test]
fn conditional_union_type_condition_keeps_only_common_members() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query($v: Boolean!) {
          rootA {
            wrapper {
              actions {
                ... on Action @skip(if: $v) {
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
        query ($v:Boolean!) {
          rootA {
            wrapper {
              actions {
                ... on Action @skip(if: $v) {
                  __typename
                  ... on Common {
                    label
                  }
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

/// Each union member asks for its own field of the same `Product` entity. One entity call for all
/// of them would ask every product for `a`, `b` and `c`, whatever member it sits under, see #1732.
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

/// Book and Movie have the same key and `title`, but `b` only resolves Book by key and `c`
/// only Movie. The fragments can't be planned once for both: each type goes to its own.
#[test]
fn fragments_on_types_resolvable_in_other_subgraphs_stay_apart() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation("{ media { ... on Book { title } ... on Movie { title } } }");
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/group-key-resolvable.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          {
            media {
              __typename
              ... on Book {
                __typename
                id
              }
              ... on Movie {
                __typename
                id
              }
            }
          }
        },
        Parallel {
          Flatten(path: "media.@|[Movie]") {
            Fetch(service: "c") {
              {
                ... on Movie {
                  __typename
                  id
                }
              } =>
              {
                ... on Movie {
                  title
                }
              }
            },
          },
          Flatten(path: "media.@|[Book]") {
            Fetch(service: "b") {
              {
                ... on Book {
                  __typename
                  id
                }
              } =>
              {
                ... on Book {
                  title
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

/// Book and Movie are both keyed by `details { code }`, but `s` hands over `code` only
/// through `Book.details`. A Movie's `code` comes from `o`, with or without the Book
/// fragment next to it.
#[test]
fn fragments_on_types_that_get_their_key_another_way_stay_apart() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation("{ media { ... on Book { title } ... on Movie { title } } }");
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/group-key-route.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "s") {
          {
            media {
              __typename
              ... on Book {
                __typename
                details {
                  code
                }
              }
              ... on Movie {
                __typename
                details {
                  __typename
                  id
                }
              }
            }
          }
        },
        Flatten(path: "media.@|[Movie].details") {
          Fetch(service: "o") {
            {
              ... on Details {
                __typename
                id
              }
            } =>
            {
              ... on Details {
                code
              }
            }
          },
        },
        BatchFetch(service: "t") {
          {
            _e0 {
              paths: [
                "media.@|[Movie]"
              ]
              {
                ... on Movie {
                  __typename
                  details {
                    code
                  }
                }
              }
            }
            _e1 {
              paths: [
                "media.@|[Book]"
              ]
              {
                ... on Book {
                  __typename
                  details {
                    code
                  }
                }
              }
            }
          }
          {
            _e0: _entities(representations: $__batch_reps_0) {
              ... on Movie {
                title
              }
            }
            _e1: _entities(representations: $__batch_reps_1) {
              ... on Book {
                title
              }
            }
          }
        },
      },
    },
    "#);

    Ok(())
}

/// `media` returns `[Book]` in a and `[Media]` in b. `Book` and `Movie` plan the same way, so
/// their fragments are planned once, for `Book` with `Movie` too. A path that's at `Book`
/// already, through a, goes straight to `title`, but one through b is at `Media` and still
/// takes `... on Book`, and `Movie` with it: `Movie`'s `title` has nowhere else to go.
#[test]
fn fragment_planned_for_several_types_keeps_them_on_a_path_at_its_type(
) -> Result<(), Box<dyn Error>> {
    init_logger();
    let supergraph = r#"
        enum join__Graph {
            A @join__graph(name: "a", url: "")
            B @join__graph(name: "b", url: "")
        }
        type Query @join__type(graph: A) @join__type(graph: B) {
            media: [Media] @join__field(graph: A, type: "[Book]") @join__field(graph: B, type: "[Media]")
        }
        union Media @join__type(graph: B) @join__unionMember(graph: B, member: "Book") @join__unionMember(graph: B, member: "Movie") = Book | Movie
        type Book @join__type(graph: A, key: "id") @join__type(graph: B, key: "id") {
            id: ID!
            title: String
            pages: Int
        }
        type Movie @join__type(graph: A, key: "id") @join__type(graph: B, key: "id") {
            id: ID!
            title: String
            length: Int
        }
    "#;
    let query_plan = crate::query_planner::tests::testkit::build_query_plan_from_sdl(
        supergraph,
        parse_operation("{ media { ... on Book { title } ... on Movie { title } } }"),
        Default::default(),
        Default::default(),
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

    // the same with one object, not a list: a's `media` is a `Book`
    let query_plan = crate::query_planner::tests::testkit::build_query_plan_from_sdl(
        &supergraph
            .replace(r#"type: "[Book]""#, r#"type: "Book""#)
            .replace(r#"type: "[Media]""#, r#"type: "Media""#)
            .replace("media: [Media]", "media: Media"),
        parse_operation("{ media { ... on Book { title } ... on Movie { title } } }"),
        Default::default(),
        Default::default(),
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

    // `Movie` written first: the fragment is planned for `Movie`, with `Book` too, and no
    // path leads to `Movie`. Each still takes `Book`, a's is at it already.
    let query_plan = crate::query_planner::tests::testkit::build_query_plan_from_sdl(
        supergraph,
        parse_operation("{ media { ... on Movie { title } ... on Book { title } } }"),
        Default::default(),
        Default::default(),
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

    let query_plan = crate::query_planner::tests::testkit::build_query_plan_from_sdl(
        supergraph,
        parse_operation("{ media { ... on Movie { title length } ... on Book { title pages } } }"),
        Default::default(),
        Default::default(),
    )?;
    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Fetch(service: "a") {
        {
          media {
            __typename
            ... on Book {
              title
              pages
            }
          }
        }
      },
    },
    "#);

    Ok(())
}

/// `r` is `[XY]` on `T1` and `[XYZ]` on `T2`. The calls for the `child` under `X` and `Y` are
/// batched for `T1` and `T2`, and `X` and `Y` are every type of `T1`'s `r`, not of `T2`'s: the
/// `[X|Y]` stays, or the call for them would also take a `Z`'s `child`, with the other currency.
#[test]
fn several_type_condition_kept_when_the_field_type_differs_per_type() -> Result<(), Box<dyn Error>>
{
    init_logger();
    let document = parse_operation(
        r#"
        {
          a {
            ... on T1 {
              r {
                ... on X { child { price(currency: "USD") } }
                ... on Y { child { price(currency: "USD") } }
              }
            }
            ... on T2 {
              r {
                ... on X { child { price(currency: "USD") } }
                ... on Y { child { price(currency: "USD") } }
                ... on Z { child { price(currency: "EUR") } }
              }
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/several-type-parent-field-types.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          {
            a {
              __typename
              ... on T1 {
                r {
                  __typename
                  ... on X {
                    ...a
                  }
                  ... on Y {
                    ...c
                  }
                }
              }
              ... on T2 {
                r {
                  __typename
                  ... on X {
                    ...a
                  }
                  ... on Y {
                    ...c
                  }
                  ... on Z {
                    child {
                      ...b
                    }
                  }
                }
              }
            }
          }
          fragment a on X {
            child {
              ...b
            }
          }
          fragment b on Child {
            __typename
            id
          }
          fragment c on Y {
            child {
              ...b
            }
          }
        },
        BatchFetch(service: "b") {
          {
            _e0 {
              paths: [
                "a.@|[T2].r.@|[Z].child"
              ]
              {
                ... on Child {
                  __typename
                  id
                }
              }
            }
            _e1 {
              paths: [
                "a.@|[T1|T2].r.@|[X|Y].child"
              ]
              {
                ... on Child {
                  __typename
                  id
                }
              }
            }
          }
          {
            _e0: _entities(representations: $__batch_reps_0) {
              ... on Child {
                price(currency: "EUR")
              }
            }
            _e1: _entities(representations: $__batch_reps_1) {
              ... on Child {
                price(currency: "USD")
              }
            }
          }
        },
      },
    },
    "#);

    Ok(())
}
