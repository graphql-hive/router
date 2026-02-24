#[cfg(test)]
mod max_depth_e2e_tests {
    use crate::testkit::{ClientResponseExt, TestRouterBuilder, TestSubgraphsBuilder};

    const QUERY: &'static str = r#"
            query {
                me {
                    name
                    reviews {
                        body
                    }
                }
            }
        "#;

    #[ntex::test]
    async fn allows_query_within_max_depth() {
        let subgraphs = TestSubgraphsBuilder::new().build().start().await;
        let router = TestRouterBuilder::new()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
        supergraph:
            source: file
            path: supergraph.graphql
        limits:
            max_depth:
                n: 3
        "#,
            )
            .build()
            .start()
            .await;

        let res = router.send_graphql_request(QUERY, None, None).await;
        insta::assert_snapshot!(res.json_body_string_pretty().await, @r###"
        {
          "data": {
            "me": {
              "name": "Uri Goldshtein",
              "reviews": [
                {
                  "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum."
                },
                {
                  "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi"
                }
              ]
            }
          }
        }
        "###);
    }

    #[ntex::test]
    async fn rejects_query_exceeding_max_depth() {
        let subgraphs = TestSubgraphsBuilder::new().build().start().await;
        let router = TestRouterBuilder::new()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
        supergraph:
            source: file
            path: supergraph.graphql
        limits:
            max_depth:
                n: 1
        "#,
            )
            .build()
            .start()
            .await;

        let res = router.send_graphql_request(QUERY, None, None).await;
        insta::assert_snapshot!(res.json_body_string_pretty().await, @r###"
        {
          "errors": [
            {
              "message": "Query depth limit exceeded.",
              "extensions": {
                "code": "MAX_DEPTH_EXCEEDED"
              }
            }
          ]
        }
        "###);
    }

    #[ntex::test]
    async fn unknown_fragments() {
        let subgraphs = TestSubgraphsBuilder::new().build().start().await;
        let router = TestRouterBuilder::new()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
        supergraph:
            source: file
            path: supergraph.graphql
        limits:
            max_depth:
                n: 3
        "#,
            )
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(
                r#"
            query {
                me {
                    ...UnknownFragment
                }
            }
            "#,
                None,
                None,
            )
            .await;
        insta::assert_snapshot!(res.json_body_string_pretty().await, @r###"
        {
          "errors": [
            {
              "message": "Unknown fragment \"UnknownFragment\".",
              "locations": [
                {
                  "line": 4,
                  "column": 24
                }
              ],
              "extensions": {
                "code": "KnownFragmentNames"
              }
            }
          ]
        }
        "###);
    }
}
