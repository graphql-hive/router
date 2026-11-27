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

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Fetch(service: "b") {
        {
          feed {
            createdAt
          }
        }
      },
    },
    "#);
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

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Parallel {
          Fetch(service: "b") {
            {
              bFeed {
                createdAt
              }
            }
          },
          Fetch(service: "a") {
            {
              aFeed {
                __typename
                id
              }
            }
          },
        },
        Flatten(path: "aFeed.@") {
          Fetch(service: "b") {
            {
              ... on Post {
                __typename
                id
              }
            } =>
            {
              ... on Post {
                createdAt
              }
            }
          },
        },
      },
    },
    "#);
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

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          {
            feed {
              __typename
              id
              ... on ImagePost {
                __typename
                id
              }
            }
          }
        },
        Flatten(path: "feed.@|[ImagePost]") {
          Fetch(service: "b") {
            {
              ... on ImagePost {
                __typename
                id
              }
            } =>
            {
              ... on ImagePost {
                createdAt
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

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Parallel {
          Fetch(service: "b") {
            {
              bFeed {
                createdAt
              }
            }
          },
          Fetch(service: "a") {
            {
              aFeed {
                __typename
                id
              }
            }
          },
        },
        Flatten(path: "aFeed.@") {
          Fetch(service: "b") {
            {
              ... on Post {
                __typename
                id
              }
            } =>
            {
              ... on Post {
                createdAt
              }
            }
          },
        },
      },
    },
    "#);

    let query_plan = build_query_plan(
        "fixture/tests/simple-progressive-overrides.supergraph.graphql",
        document,
        // @override(label: "percentage(75)")
        PlannerOverrideContext::from_percentage(90.0),
        Default::default(),
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Parallel {
          Fetch(service: "b") {
            {
              bFeed {
                __typename
                id
              }
            }
          },
          Fetch(service: "a") {
            {
              aFeed {
                createdAt
              }
            }
          },
        },
        Flatten(path: "bFeed.@") {
          Fetch(service: "a") {
            {
              ... on Post {
                __typename
                id
              }
            } =>
            {
              ... on Post {
                createdAt
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

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Fetch(service: "b") {
        {
          feed {
            id
          }
        }
      },
    },
    "#);

    let query_plan = build_query_plan(
        "fixture/tests/simple-progressive-overrides.supergraph.graphql",
        document,
        // @override(label: "feed_in_b")
        PlannerOverrideContext::from_flag("different_flag".into()),
        Default::default(),
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Fetch(service: "a") {
        {
          feed {
            id
          }
        }
      },
    },
    "#);
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

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Fetch(service: "b") {
        {
          me {
            media {
              __typename
              ... on Book {
                id
              }
              ... on Movie {
                id
              }
            }
          }
        }
      },
    },
    "#);

    // flag off: a still owns `media`
    let query_plan = build_query_plan(
        "fixture/tests/union-progressive-override.supergraph.graphql",
        document,
        PlannerOverrideContext::from_flag("different_flag".into()),
        Default::default(),
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "b") {
          {
            me {
              __typename
              id
            }
          }
        },
        Flatten(path: "me") {
          Fetch(service: "a") {
            {
              ... on User {
                __typename
                id
              }
            } =>
            {
              ... on User {
                media {
                  __typename
                  ... on Book {
                    id
                  }
                  ... on Movie {
                    id
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

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Fetch(service: "b") {
        {
          me {
            orders {
              sku
            }
          }
        }
      },
    },
    "#);

    // flag off: a still owns `orders`, so b's @provides path can't be used
    let query_plan = build_query_plan(
        "fixture/tests/provides-progressive-override.supergraph.graphql",
        document,
        PlannerOverrideContext::from_flag("different_flag".into()),
        Default::default(),
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "b") {
          {
            me {
              __typename
              id
            }
          }
        },
        Flatten(path: "me") {
          Fetch(service: "a") {
            {
              ... on User {
                __typename
                id
              }
            } =>
            {
              ... on User {
                orders {
                  sku
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

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Fetch(service: "b") {
        {
          me {
            media {
              __typename
              ... on Book {
                title
              }
              ... on Movie {
                title
              }
            }
          }
        }
      },
    },
    "#);

    // flag off: a still owns `media`
    let query_plan = build_query_plan(
        "fixture/tests/provides-union-progressive-override.supergraph.graphql",
        document,
        PlannerOverrideContext::from_flag("different_flag".into()),
        Default::default(),
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "b") {
          {
            me {
              __typename
              id
            }
          }
        },
        Flatten(path: "me") {
          Fetch(service: "a") {
            {
              ... on User {
                __typename
                id
              }
            } =>
            {
              ... on User {
                media {
                  __typename
                  ... on Book {
                    title
                  }
                  ... on Movie {
                    title
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

const OVERRIDDEN_MEMBER_FIELD: &str =
    "fixture/tests/interface-object-requires-overridden-member-field.supergraph.graphql";
const PROGRESSIVELY_OVERRIDDEN_MEMBER_FIELD: &str =
    "fixture/tests/interface-object-requires-progressively-overridden-member-field.supergraph.graphql";

/// `Account.title` is in `first`, but `Admin.title` moved to `third` with `@override`. Reading the
/// interface's field reads it for an `Admin` from `third`.
#[test]
fn override_of_interface_member_field() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation("{ accounts { title } }");
    let query_plan = build_query_plan_with_defaults(OVERRIDDEN_MEMBER_FIELD, document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "first") {
          {
            accounts {
              __typename
              ... on Admin {
                __typename
                id
              }
              ... on Moderator {
                title
              }
            }
          }
        },
        Flatten(path: "accounts.@|[Admin]") {
          Fetch(service: "third") {
            {
              ... on Admin {
                __typename
                id
              }
            } =>
            {
              ... on Admin {
                title
              }
            }
          },
        },
      },
    },
    "#);

    Ok(())
}

/// The same with `@override(label: "percent(50)")`: `third` for a request in the 50%, `first` for
/// the rest.
#[test]
fn progressive_override_of_interface_member_field() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation("{ accounts { title } }");

    let in_range = build_query_plan(
        PROGRESSIVELY_OVERRIDDEN_MEMBER_FIELD,
        document.clone(),
        PlannerOverrideContext::from_percentage(0.0),
        Default::default(),
    )?;
    insta::assert_snapshot!(format!("{}", in_range), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "first") {
          {
            accounts {
              __typename
              ... on Admin {
                __typename
                id
              }
              ... on Moderator {
                title
              }
            }
          }
        },
        Flatten(path: "accounts.@|[Admin]") {
          Fetch(service: "third") {
            {
              ... on Admin {
                __typename
                id
              }
            } =>
            {
              ... on Admin {
                title
              }
            }
          },
        },
      },
    },
    "#);

    let out_of_range = build_query_plan(
        PROGRESSIVELY_OVERRIDDEN_MEMBER_FIELD,
        document,
        PlannerOverrideContext::from_percentage(90.0),
        Default::default(),
    )?;
    insta::assert_snapshot!(format!("{}", out_of_range), @r#"
    QueryPlan {
      Fetch(service: "first") {
        {
          accounts {
            __typename
            title
          }
        }
      },
    },
    "#);

    Ok(())
}

/// Asking for the field on the type itself, under its interface, follows the override too.
#[test]
fn progressive_override_of_interface_member_field_in_fragment() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation("{ accounts { ... on Admin { title } } }");
    let query_plan = build_query_plan(
        PROGRESSIVELY_OVERRIDDEN_MEMBER_FIELD,
        document,
        PlannerOverrideContext::from_percentage(0.0),
        Default::default(),
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "first") {
          {
            accounts {
              __typename
              ... on Admin {
                __typename
                id
              }
            }
          }
        },
        Flatten(path: "accounts.@|[Admin]") {
          Fetch(service: "third") {
            {
              ... on Admin {
                __typename
                id
              }
            } =>
            {
              ... on Admin {
                title
              }
            }
          },
        },
      },
    },
    "#);

    Ok(())
}

/// `Admin.scope: Query` moved from `first` to `third`, and `version` is only in `fourth`. Going
/// back to `Query` through `scope` starts from `third`, not from `first` that no longer has it.
#[test]
fn override_of_field_that_reenters_root() -> Result<(), Box<dyn Error>> {
    init_logger();
    let fixture = "fixture/tests/override-root-reentry.supergraph.graphql";

    let query_plan = build_query_plan_with_defaults(
        fixture,
        parse_operation("{ admin { scope { version } } }"),
    )?;
    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "first") {
          {
            admin {
              __typename
              id
            }
          }
        },
        Flatten(path: "admin") {
          Fetch(service: "third") {
            {
              ... on Admin {
                __typename
                id
              }
            } =>
            {
              ... on Admin {
                scope {
                  __typename
                }
              }
            }
          },
        },
        Flatten(path: "admin.scope") {
          Fetch(service: "fourth") {
            {
              version
            }
          },
        },
      },
    },
    "#);

    // through the interface, only the Admins' `scope` moved
    let query_plan = build_query_plan_with_defaults(
        fixture,
        parse_operation("{ accounts { scope { version } } }"),
    )?;
    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "first") {
          {
            accounts {
              __typename
              ... on Admin {
                __typename
                id
              }
              ... on Moderator {
                scope {
                  __typename
                }
              }
            }
          }
        },
        Parallel {
          Flatten(path: "accounts.@|[Moderator].scope") {
            Fetch(service: "fourth") {
              {
                version
              }
            },
          },
          Flatten(path: "accounts.@|[Admin]") {
            Fetch(service: "third") {
              {
                ... on Admin {
                  __typename
                  id
                }
              } =>
              {
                ... on Admin {
                  scope {
                    __typename
                  }
                }
              }
            },
          },
        },
        Flatten(path: "accounts.@|[Admin].scope") {
          Fetch(service: "fourth") {
            {
              version
            }
          },
        },
      },
    },
    "#);

    Ok(())
}

/// The same with `@override(label: "percent(50)")`: `third` for a request in the 50%, `first` for
/// the rest.
#[test]
fn progressive_override_of_field_that_reenters_root() -> Result<(), Box<dyn Error>> {
    init_logger();
    let fixture = "fixture/tests/progressive-override-root-reentry.supergraph.graphql";
    let document = parse_operation("{ admin { scope { version } } }");

    let in_range = build_query_plan(
        fixture,
        document.clone(),
        PlannerOverrideContext::from_percentage(0.0),
        Default::default(),
    )?;
    insta::assert_snapshot!(format!("{}", in_range), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "first") {
          {
            admin {
              __typename
              id
            }
          }
        },
        Flatten(path: "admin") {
          Fetch(service: "third") {
            {
              ... on Admin {
                __typename
                id
              }
            } =>
            {
              ... on Admin {
                scope {
                  __typename
                }
              }
            }
          },
        },
        Flatten(path: "admin.scope") {
          Fetch(service: "fourth") {
            {
              version
            }
          },
        },
      },
    },
    "#);

    let out_of_range = build_query_plan(
        fixture,
        document,
        PlannerOverrideContext::from_percentage(90.0),
        Default::default(),
    )?;
    insta::assert_snapshot!(format!("{}", out_of_range), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "first") {
          {
            admin {
              scope {
                __typename
              }
            }
          }
        },
        Flatten(path: "admin.scope") {
          Fetch(service: "fourth") {
            {
              version
            }
          },
        },
      },
    },
    "#);

    // through the interface, only the Admins' `scope` moved, for a request in the 50%
    let query_plan = build_query_plan(
        fixture,
        parse_operation("{ accounts { scope { version } } }"),
        PlannerOverrideContext::from_percentage(0.0),
        Default::default(),
    )?;
    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "first") {
          {
            accounts {
              __typename
              ... on Admin {
                __typename
                id
              }
              ... on Moderator {
                scope {
                  __typename
                }
              }
            }
          }
        },
        Parallel {
          Flatten(path: "accounts.@|[Moderator].scope") {
            Fetch(service: "fourth") {
              {
                version
              }
            },
          },
          Flatten(path: "accounts.@|[Admin]") {
            Fetch(service: "third") {
              {
                ... on Admin {
                  __typename
                  id
                }
              } =>
              {
                ... on Admin {
                  scope {
                    __typename
                  }
                }
              }
            },
          },
        },
        Flatten(path: "accounts.@|[Admin].scope") {
          Fetch(service: "fourth") {
            {
              version
            }
          },
        },
      },
    },
    "#);

    Ok(())
}
