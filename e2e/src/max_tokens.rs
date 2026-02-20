#[cfg(test)]
mod max_tokens_e2e_tests {
    use sonic_rs::{to_string_pretty, Value};

    use crate::testkit_v2::{TestRouterBuilder, TestSubgraphsBuilder};

    static QUERY: &str = r#"
        query {
            me {
                id
            }
        }
    "#;

    #[ntex::test]
    async fn does_not_reject_an_operation_below_token_limit() {
        let subgraphs = TestSubgraphsBuilder::new().build().start().await;
        let router = TestRouterBuilder::new()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
            supergraph:
                source: file
                path: ./supergraph.graphql
            limits:
                max_tokens:
                    n: 100
            "#,
            )
            .build()
            .start()
            .await;

        let res = router.send_graphql_request(QUERY, None, None).await;

        let body = res.body().await.unwrap();
        let json_body: Value = sonic_rs::from_slice(&body).unwrap();
        insta::assert_snapshot!(to_string_pretty(&json_body).unwrap(), @r#"
        {
          "data": {
            "me": {
              "id": "1"
            }
          }
        }
        "#);
    }

    #[ntex::test]
    async fn rejects_an_operation_exceeding_token_limit() {
        let subgraphs = TestSubgraphsBuilder::new().build().start().await;
        let router = TestRouterBuilder::new()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
            supergraph:
                source: file
                path: ./supergraph.graphql
            limits:
                max_tokens:
                    n: 4
            "#,
            )
            .build()
            .start()
            .await;

        let res = router.send_graphql_request(QUERY, None, None).await;
        let body = res.body().await.unwrap();
        let json_body: Value = sonic_rs::from_slice(&body).unwrap();

        insta::assert_snapshot!(to_string_pretty(&json_body).unwrap(), @r#"
        {
          "errors": [
            {
              "message": "Token limit exceeded.",
              "locations": [
                {
                  "line": 4,
                  "column": 17
                }
              ],
              "extensions": {
                "code": "TOKEN_LIMIT_EXCEEDED"
              }
            }
          ]
        }
        "#);
    }
}
