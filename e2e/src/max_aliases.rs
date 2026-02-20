#[cfg(test)]
mod max_aliases_e2e_tests {
    use sonic_rs::{to_string_pretty, Value};

    use crate::testkit_v2::{TestRouterBuilder, TestSubgraphsBuilder};

    #[ntex::test]
    async fn allows_query_within_max_aliases() -> Result<(), Box<dyn std::error::Error>> {
        let subgraphs = TestSubgraphsBuilder::new().build().start().await;
        let router = TestRouterBuilder::new()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
        supergraph:
            source: file
            path: supergraph.graphql
        limits:
            max_aliases:
                n: 3
        "#,
            )
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(
                "query { 
                myInfo: me {
                    myName: name
                }
            }",
                None,
                None,
            )
            .await;
        assert!(res.status().is_success(), "Expected 200 OK");

        let body = res.body().await.unwrap();
        let json_body: Value = sonic_rs::from_slice(&body)?;
        insta::assert_snapshot!(to_string_pretty(&json_body)?, @r###"
        {
          "data": {
            "myInfo": {
              "myName": "Uri Goldshtein"
            }
          }
        }
        "###);
        Ok(())
    }

    #[ntex::test]
    async fn rejects_query_exceeding_max_aliases() -> Result<(), Box<dyn std::error::Error>> {
        let subgraphs = TestSubgraphsBuilder::new().build().start().await;
        let router = TestRouterBuilder::new()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
        supergraph:
            source: file
            path: supergraph.graphql
        limits:
            max_aliases:
                n: 3
        "#,
            )
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(
                "query { 
                typeName: __typename
                userInfo: me {
                    userName: name
                    userReviews: reviews {
                        reviewTypeName: __typename
                    }
                }
            }",
                None,
                None,
            )
            .await;

        let body = res.body().await.unwrap();
        let json_body: Value = sonic_rs::from_slice(&body)?;

        insta::assert_snapshot!(to_string_pretty(&json_body)?, @r###"
        {
          "errors": [
            {
              "message": "Aliases limit exceeded.",
              "extensions": {
                "code": "MAX_ALIASES_EXCEEDED"
              }
            }
          ]
        }
        "###);

        Ok(())
    }
}
