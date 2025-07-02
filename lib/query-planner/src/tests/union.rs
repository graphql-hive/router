use crate::{
    tests::testkit::{build_query_plan, init_logger},
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
    let query_plan = build_query_plan(
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
    let query_plan = build_query_plan(
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
    let query_plan = build_query_plan(
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
    let query_plan = build_query_plan(
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
                aTitle
                id
                title
              }
            }
          }
        },
        Flatten(path: "aMedia") {
          Fetch(service: "b") {
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
    let query_plan = build_query_plan(
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
                ... on Book {
                  __typename
                  aTitle
                  id
                  title
                }
                ... on Song {
                  aTitle
                  title
                }
              }
            }
          }
        },
        Flatten(path: "viewer.song") {
          Fetch(service: "b") {
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
    let query_plan = build_query_plan(
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
                book {
    ...a            }
                media {
    ...a            }
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
                book {
    ...a            }
                media {
    ...a            }
                song {
                  __typename
                  ... on Book {
                    __typename
                    aTitle
                    id
                    title
                  }
                  ... on Song {
                    aTitle
                    title
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
        Flatten(path: "viewer.song") {
          Fetch(service: "b") {
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
