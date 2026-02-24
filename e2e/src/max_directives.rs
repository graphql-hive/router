#[cfg(test)]
mod max_directives_e2e_tests {
    use sonic_rs::{to_string_pretty, Value};

    use crate::testkit::{TestRouterBuilder, TestSubgraphsBuilder};

    #[ntex::test]
    async fn allows_query_within_max_directives() {
        let subgraphs = TestSubgraphsBuilder::new().build().start().await;
        let router = TestRouterBuilder::new()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
        supergraph:
            source: file
            path: supergraph.graphql
        limits:
            max_directives:
                n: 8
        "#,
            )
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(
                "query {
                __typename @include(if: true)
                me @skip(if: false) {
                    __typename @include(if: true)
                    name @skip(if: false)
                    reviews @include(if: true) {
                        __typename @skip(if: false)
                    }
                }
            }",
                None,
                None,
            )
            .await;
        assert!(res.status().is_success(), "Expected 200 OK");

        let body = res.body().await.unwrap();
        let json_body: Value = sonic_rs::from_slice(&body).unwrap();
        insta::assert_snapshot!(to_string_pretty(&json_body).unwrap(), @r###"
        {
          "data": {
            "__typename": "Query",
            "me": {
              "__typename": "User",
              "name": "Uri Goldshtein",
              "reviews": [
                {
                  "__typename": "Review"
                },
                {
                  "__typename": "Review"
                }
              ]
            }
          }
        }
        "###);
    }

    #[ntex::test]
    async fn rejects_query_exceeding_max_directives() {
        let subgraphs = TestSubgraphsBuilder::new().build().start().await;
        let router = TestRouterBuilder::new()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
        supergraph:
            source: file
            path: supergraph.graphql
        limits:
            max_directives:
                n: 5
        "#,
            )
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(
                "query {
                __typename @include(if: true)
                me @skip(if: false) {
                    __typename @include(if: true)
                    name @skip(if: false)
                    reviews @include(if: true) {
                        __typename @skip(if: false)
                    }
                }
            }",
                None,
                None,
            )
            .await;

        let body = res.body().await.unwrap();
        let json_body: Value = sonic_rs::from_slice(&body).unwrap();

        insta::assert_snapshot!(to_string_pretty(&json_body).unwrap(), @r###"
        {
          "errors": [
            {
              "message": "Directives limit exceeded.",
              "extensions": {
                "code": "MAX_DIRECTIVES_EXCEEDED"
              }
            }
          ]
        }
        "###);
    }
}
