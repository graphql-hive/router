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
    let query_plan = build_query_plan(
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
