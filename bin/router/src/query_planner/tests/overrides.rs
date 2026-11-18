use crate::query_planner::{
    graph::PlannerOverrideContext,
    tests::testkit::{build_query_plan, build_query_plan_with_defaults, init_logger},
    utils::parsing::parse_operation,
};
use std::error::Error;

#[test]
fn single_simple_overrides() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          feed {
            createdAt
          }
        }"#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/simple_overrides.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] b               -

    [2] b
      out
        { feed { createdAt } }
    ");
    insta::assert_snapshot!(format!("{}", sonic_rs::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Fetch",
        "serviceName": "b",
        "operationKind": "query",
        "operation": "{feed{createdAt}}"
      }
    }
    "#);
    Ok(())
}

#[test]
fn two_fields_simple_overrides() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          aFeed {
            createdAt
          }
          bFeed {
            createdAt
          }
        }"#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/simple_overrides.supergraph.graphql",
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
        { bFeed { createdAt } }

    [2] a
      out
        { aFeed { __typename id } }

    [3] b   after [2]
      merge
        aFeed.@
      in
        ... on Post { __typename id }
      out
        ... on Post { createdAt }
    ");
    insta::assert_snapshot!(format!("{}", sonic_rs::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Sequence",
        "nodes": [
          {
            "kind": "Parallel",
            "nodes": [
              {
                "kind": "Fetch",
                "serviceName": "b",
                "operationKind": "query",
                "operation": "{bFeed{createdAt}}"
              },
              {
                "kind": "Fetch",
                "serviceName": "a",
                "operationKind": "query",
                "operation": "{aFeed{__typename id}}"
              }
            ]
          },
          {
            "kind": "Flatten",
            "path": [
              {
                "Field": "aFeed"
              },
              "@"
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "b",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Post{createdAt}}}",
              "requires": [
                {
                  "kind": "InlineFragment",
                  "typeCondition": "Post",
                  "selections": [
                    {
                      "kind": "Field",
                      "name": "__typename"
                    },
                    {
                      "kind": "Field",
                      "name": "id"
                    }
                  ]
                }
              ]
            }
          }
        ]
      }
    }
    "#);
    Ok(())
}

#[test]
fn override_object_field_but_interface_is_requested() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          feed {
            id
            createdAt
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/override-type-interface.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] a               -
      [3] b               [2]

    [2] a
      out
        { feed { __typename id ... on ImagePost { __typename id } } }

    [3] b   after [2]
      merge
        feed.@|[ImagePost]
      in
        ... on ImagePost { __typename id }
      out
        ... on ImagePost { createdAt }
    ");
    Ok(())
}

#[test]
fn progressive_override_percentage_test() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          aFeed {
            createdAt
          }
          bFeed {
            createdAt
          }
        }
        "#,
    );
    let query_plan = build_query_plan(
        "fixture/tests/simple-progressive-overrides.supergraph.graphql",
        document.clone(),
        // @override(label: "percentage(75)")
        PlannerOverrideContext::from_percentage(50.0),
        Default::default(),
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [4] b               -
      [2] a               -
      [3] b               [2]

    [4] b
      out
        { bFeed { createdAt } }

    [2] a
      out
        { aFeed { __typename id } }

    [3] b   after [2]
      merge
        aFeed.@
      in
        ... on Post { __typename id }
      out
        ... on Post { createdAt }
    ");

    let query_plan = build_query_plan(
        "fixture/tests/simple-progressive-overrides.supergraph.graphql",
        document,
        // @override(label: "percentage(75)")
        PlannerOverrideContext::from_percentage(90.0),
        Default::default(),
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [3] b               -
      [2] a               -
      [4] a               [3]

    [3] b
      out
        { bFeed { __typename id } }

    [2] a
      out
        { aFeed { createdAt } }

    [4] a   after [3]
      merge
        bFeed.@
      in
        ... on Post { __typename id }
      out
        ... on Post { createdAt }
    ");
    Ok(())
}

#[test]
fn progressive_override_flag_test() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          feed {
            id
          }
        }
        "#,
    );
    let query_plan = build_query_plan(
        "fixture/tests/simple-progressive-overrides.supergraph.graphql",
        document.clone(),
        // @override(label: "feed_in_b")
        PlannerOverrideContext::from_flag("feed_in_b".into()),
        Default::default(),
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] b               -

    [2] b
      out
        { feed { id } }
    ");

    let query_plan = build_query_plan(
        "fixture/tests/simple-progressive-overrides.supergraph.graphql",
        document,
        // @override(label: "feed_in_b")
        PlannerOverrideContext::from_flag("different_flag".into()),
        Default::default(),
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] a               -

    [2] a
      out
        { feed { id } }
    ");
    Ok(())
}

/// `b` takes `User.media`, a union field, from `a` behind a label. Each union member has its own
/// edge, and the label has to be on all of them, or `b` keeps serving `media` with the flag off.
#[test]
fn progressive_override_of_union_field() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          me {
            media {
              ... on Book {
                id
              }
              ... on Movie {
                id
              }
            }
          }
        }
        "#,
    );

    // flag on: `media` comes from b
    let query_plan = build_query_plan(
        "fixture/tests/union-progressive-override.supergraph.graphql",
        document.clone(),
        PlannerOverrideContext::from_flag("media_in_b".into()),
        Default::default(),
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] b               -

    [2] b
      out
        { me { media { __typename ... on Book { id } ... on Movie { id } } } }
    ");

    // flag off: a still owns `media`
    let query_plan = build_query_plan(
        "fixture/tests/union-progressive-override.supergraph.graphql",
        document,
        PlannerOverrideContext::from_flag("different_flag".into()),
        Default::default(),
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] b               -
      [3] a               [2]

    [2] b
      out
        { me { __typename id } }

    [3] a   after [2]
      merge
        me
      in
        ... on User { __typename id }
      out
        ... on User { media { __typename ... on Book { id } ... on Movie { id } } }
    ");

    Ok(())
}

/// `b` takes `User.orders` from `a` behind a label, and provides `sku` on it. The provided `sku`
/// only counts when `orders` really comes from `b`.
#[test]
fn progressive_override_of_provides_field() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          me {
            orders {
              sku
            }
          }
        }
        "#,
    );
    // flag on: `orders` comes from b, and so does the provided `sku`
    let query_plan = build_query_plan(
        "fixture/tests/provides-progressive-override.supergraph.graphql",
        document.clone(),
        PlannerOverrideContext::from_flag("orders_in_b".into()),
        Default::default(),
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] b               -

    [2] b
      out
        { me { orders { sku } } }
    ");

    // flag off: a still owns `orders`, so b's @provides path can't be used
    let query_plan = build_query_plan(
        "fixture/tests/provides-progressive-override.supergraph.graphql",
        document,
        PlannerOverrideContext::from_flag("different_flag".into()),
        Default::default(),
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] b               -
      [3] a               [2]

    [2] b
      out
        { me { __typename id } }

    [3] a   after [2]
      merge
        me
      in
        ... on User { __typename id }
      out
        ... on User { orders { sku } }
    ");

    Ok(())
}

/// Same as above, with a union field. `b` takes `User.media` from `a` behind a label and
/// provides `title` on both members. Each member has its own edge, and the label has to be on
/// all of them.
#[test]
fn progressive_override_of_provides_union_field() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          me {
            media {
              ... on Book {
                title
              }
              ... on Movie {
                title
              }
            }
          }
        }
        "#,
    );

    // flag on: `media` comes from b, and so do the provided titles
    let query_plan = build_query_plan(
        "fixture/tests/provides-union-progressive-override.supergraph.graphql",
        document.clone(),
        PlannerOverrideContext::from_flag("media_in_b".into()),
        Default::default(),
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] b               -

    [2] b
      out
        { me { media { __typename ... on Book { title } ... on Movie { title } } } }
    ");

    // flag off: a still owns `media`
    let query_plan = build_query_plan(
        "fixture/tests/provides-union-progressive-override.supergraph.graphql",
        document,
        PlannerOverrideContext::from_flag("different_flag".into()),
        Default::default(),
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] b               -
      [3] a               [2]

    [2] b
      out
        { me { __typename id } }

    [3] a   after [2]
      merge
        me
      in
        ... on User { __typename id }
      out
        ... on User { media { __typename ... on Book { title } ... on Movie { title } } }
    ");

    Ok(())
}
