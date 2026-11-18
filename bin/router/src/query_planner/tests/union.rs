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

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] a               -

    [2] a
      out
        { media { __typename ... on Book { title } } }
    ");

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

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] a               -

    [2] a
      out
        { media { __typename } }
    ");

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

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] a               -

    [2] a
      out
        { media { __typename ... on Book { title } } }
    ");

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

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] a               -
      [3] b               [2]

    [2] a
      out
        { aMedia { __typename ... on Book { __typename title aTitle id } } }

    [3] b   after [2]
      merge
        aMedia|[Book]
      in
        ... on Book { __typename id }
      out
        ... on Book { bTitle }
    ");

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

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] a               -
      [3] b               [2]

    [2] a
      out
        {
          viewer {
            song { __typename ... on Song { title aTitle } ... on Book { __typename title aTitle id } }
          }
        }

    [3] b   after [2]
      merge
        viewer.song|[Book]
      in
        ... on Book { __typename id }
      out
        ... on Book { bTitle }
    ");

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

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [4] b               -
      [2] a               -
      [3] b               [2]

    [4] b
      out
        { viewer { media { ...a } book { ...a } } }
        fragment a on ViewerMedia { __typename ... on Book { bTitle } }

    [2] a
      out
        {
          viewer {
            media { ...a }
            book { ...a }
            song { __typename ... on Song { title aTitle } ... on Book { __typename title aTitle id } }
          }
        }
        fragment a on ViewerMedia { __typename ... on Book { aTitle title } }

    [3] b   after [2]
      merge
        viewer.song|[Book]
      in
        ... on Book { __typename id }
      out
        ... on Book { bTitle }
    ");

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

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] a               -

    [2] a
      out
        {
          getResponse {
            message
            actions {
              __typename
              ... on Alpha { id value }
              ... on Beta { id name details }
              ... on Gamma { id label }
            }
          }
        }
    ");

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

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] a               -

    [2] a
      out
        { getResponse { actions { __typename ... on Beta { name } ... on Gamma { label } } } }
    ");

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

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] a               -

    [2] a
      out
        { rootA { wrapper { actions { __typename ... on Common { label } } } } }
    ");

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

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] b               -

    [2] b
      out
        { rootB { wrapper { actions { __typename ... on Common { label } } } } }
    ");

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

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] a               -

    [2] a
      out
        { rootA { wrapper { actions { __typename } } } }
    ");

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

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] a               -

    [2] a
      out
        { shared { wrapper { actions { __typename ... on Common { label } } } } }
    ");

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

    insta::assert_snapshot!(query_plan, @r"
    fetches
      fetch               after
      [2] a               -
      [3] b               [2]

    [2] a
      out
        { rootA { __typename id } }

    [3] b   after [2]
      merge
        rootA
      in
        ... on Container { __typename id }
      out
        ... on Container {
          bWrapper { actions { __typename ... on Common { label } ... on OnlyB { b } } }
        }
    ");

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

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] a               -
      [5] d               [2]
      [4] c               [2]
      [3] b               [2]

    [2] a
      out
        {
          review {
            __typename
            ... on AnonymousReview { product { ...a } }
            ... on UserReview { product { ...a } }
          }
        }
        fragment a on Product { __typename id }

    [5] d   after [2]
      merge
        review|[UserReview].product
      in
        ... on Product { __typename id }
      out
        ... on Product { d }

    [4] c   after [2]
      merge
        review|[UserReview].product
      in
        ... on Product { __typename id }
      out
        ... on Product { c }

    [3] b   after [2]
      merge
        review|[AnonymousReview].product
      in
        ... on Product { __typename id }
      out
        ... on Product { b }
    ");

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

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] a               -
      [4] b               [2]

    [2] a
      out
        {
          orders {
            __typename
            ... on OrdersPageConnections {
              items {
                id
                items {
                  __typename
                  ... on PaperBook { product { ...a } }
                  ... on DigitalBook { product { ...a } }
                }
              }
            }
          }
        }
        fragment a on Product { __typename id }

    [4] b   after [2]
      merge
        orders|[OrdersPageConnections].items.@.items.@.product
      in
        ... on Product { __typename id }
      out
        ... on Product { name slug }
    ");

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

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    variables
      $v: Boolean!

    fetches
      fetch               after
      [2] card            -
      [4] product         [2]

    [2] card
      out
        {
          card {
            components {
              ... on Component @include(if: $v) {
                __typename
                ... on ComponentA { product { ...a } }
                ... on ComponentC { product { ...a } }
              }
            }
          }
        }
        fragment a on Product { __typename id }
      vars
        $v

    [4] product   after [2]
      alias _e0
        if
          $v
        merge
          card.components.@|[Component]|[ComponentC].product
        in
          ... on Product { __typename id }
        out
          ... on Product { c }
      alias _e1
        if
          $v
        merge
          card.components.@|[Component]|[ComponentA].product
        in
          ... on Product { __typename id }
        out
          ... on Product { a }
    ");

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

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    variables
      $a: Boolean!, $b: Boolean!

    fetches
      fetch               after
      [2] card            -
      [3] product         [2]

    [2] card
      out
        {
          card {
            components {
              ... on Component @skip(if: $a) {
                __typename
                ... on ComponentC { product @skip(if: $b) { __typename id } }
              }
            }
          }
        }
      vars
        $a, $b

    [3] product   after [2]
      if
        !$a && !$b
      merge
        card.components.@|[Component]|[ComponentC].product
      in
        ... on Product { __typename id }
      out
        ... on Product { b }
    ");

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

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    variables
      $v: Boolean!

    fetches
      fetch               after
      [2] a               -

    [2] a
      out
        {
          rootA {
            wrapper { actions { ... on Action @skip(if: $v) { __typename ... on Common { label } } } }
          }
        }
      vars
        $v
    ");

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

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] card            -
      [5] product         [2]

    [2] card
      out
        {
          card {
            components {
              __typename
              ... on ComponentA { product { ...a } }
              ... on ComponentB { product { ...a } }
              ... on ComponentC { product { ...a } }
            }
          }
        }
        fragment a on Product { __typename id }

    [5] product   after [2]
      alias _e0
        merge
          card.components.@|[ComponentC].product
        in
          ... on Product { __typename id }
        out
          ... on Product { c }
      alias _e1
        merge
          card.components.@|[ComponentB].product
        in
          ... on Product { __typename id }
        out
          ... on Product { b }
      alias _e2
        merge
          card.components.@|[ComponentA].product
        in
          ... on Product { __typename id }
        out
          ... on Product { a }
    ");

    Ok(())
}
