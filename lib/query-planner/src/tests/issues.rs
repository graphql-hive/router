use crate::{
    tests::testkit::{build_query_plan, init_logger},
    utils::parsing::parse_operation,
};
use std::error::Error;

#[test]
fn issue_281_test() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        {
          viewer {
            review {
              ... on AnonymousReview {
                __typename
                product {
                  b
                }
              }
              ... on UserReview {
                __typename
                product {
                  c
                  d
                }
              }
            }
          }
        }

        "#,
    );
    let query_plan = build_query_plan("fixture/issues/281.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          {
            viewer {
              review {
                __typename
                ... on AnonymousReview {
                  __typename
                  product {
                    ...a
                  }
                }
                ... on UserReview {
                  __typename
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
        BatchFetch(service: "b") {
          {
            _e0 {
              paths: [
                "viewer.review|[UserReview].product"
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
                "viewer.review|[AnonymousReview].product"
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
                pid
              }
            }
            _e1: _entities(representations: $__batch_reps_1) {
              ... on Product {
                b
              }
            }
          }
        },
        Flatten(path: "viewer.review|[UserReview].product") {
          Fetch(service: "c") {
            {
              ... on Product {
                __typename
                pid
              }
            } =>
            {
              ... on Product {
                c
                pid
              }
            }
          },
        },
        Flatten(path: "viewer.review|[UserReview].product") {
          Fetch(service: "d") {
            {
              ... on Product {
                __typename
                pid
              }
            } =>
            {
              ... on Product {
                d
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
fn issue_190_test() -> Result<(), Box<dyn Error>> {
    init_logger();

    // Original version
    let document = parse_operation(
        r#"
        query(
          $included: Boolean!
        ) {
          recommender @include(if: $included) {
            id
            results {
              ...Recommendable_Product
              __typename
            }
            __typename
          }
        }

        fragment Recommendable_Product on Product {
          id
        }
      "#,
    );
    let query_plan = build_query_plan("fixture/issues/190.supergraph.graphql", document)?;
    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Fetch(service: "recommender") {
        query ($included:Boolean!) {
          recommender @include(if: $included) {
            id
            results {
              __typename
              ... on Product {
                id
              }
            }
            __typename
          }
        }
      },
    },
    "#);

    // Without __typename version
    let document = parse_operation(
        r#"
        query(
          $included: Boolean!
        ) {
          recommender @include(if: $included) {
            id
            results {
              ...Recommendable_Product
            }
          }
        }

        fragment Recommendable_Product on Product {
          id
        }
      "#,
    );
    let query_plan = build_query_plan("fixture/issues/190.supergraph.graphql", document)?;
    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Fetch(service: "recommender") {
        query ($included:Boolean!) {
          recommender @include(if: $included) {
            id
            results {
              __typename
              ... on Product {
                id
              }
            }
          }
        }
      },
    },
    "#);

    // Inline fragment version
    let document = parse_operation(
        r#"
        query(
          $included: Boolean!
        ) {
          recommender @include(if: $included) {
            id
            results {
              ... on Product {
                id
              }
            }
          }
        }
      "#,
    );
    let query_plan = build_query_plan("fixture/issues/190.supergraph.graphql", document)?;
    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Fetch(service: "recommender") {
        query ($included:Boolean!) {
          recommender @include(if: $included) {
            id
            results {
              __typename
              ... on Product {
                id
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
fn issue_939_test() -> Result<(), Box<dyn Error>> {
    init_logger();

    let named_fragment_document = parse_operation(
        r#"
        query SingleNode($id: ID!) {
          node(id: $id) {
            ... on MyNode {
              content {
                ... on ITextContent {
                  fragments {
                    contentNode {
                      content {
                        ...ITextContentPreview
                      }
                    }
                  }
                }
              }
            }
          }
        }

        fragment ITextContentPreview on ITextContent {
          id
        }
        "#,
    );
    let named_fragment_plan = build_query_plan(
        "fixture/issues/939.supergraph.graphql",
        named_fragment_document,
    )?;

    let inline_fragment_document = parse_operation(
        r#"
        query SingleNode($id: ID!) {
          node(id: $id) {
            ... on MyNode {
              content {
                ... on ITextContent {
                  fragments {
                    contentNode {
                      content {
                        ... on ITextContent {
                          id
                        }
                      }
                    }
                  }
                }
              }
            }
          }
        }
        "#,
    );
    let inline_fragment_plan = build_query_plan(
        "fixture/issues/939.supergraph.graphql",
        inline_fragment_document,
    )?;

    assert_eq!(
        format!("{}", named_fragment_plan),
        format!("{}", inline_fragment_plan)
    );

    insta::assert_snapshot!(format!("{}", inline_fragment_plan), @r#"
    QueryPlan {
      Fetch(service: "content") {
        query ($id:ID!) {
          node(id: $id) {
            __typename
            ... on MyNode {
              content {
                __typename
                ... on TextContent {
                  fragments {
                    ...a
                  }
                }
                ... on TextGroupContent {
                  fragments {
                    ...a
                  }
                }
              }
            }
          }
        }
        fragment a on TextContentFragment {
          contentNode {
            content {
              __typename
              ... on TextContent {
                id
              }
              ... on TextGroupContent {
                id
              }
            }
          }
        }
      },
    },
    "#);

    Ok(())
}
