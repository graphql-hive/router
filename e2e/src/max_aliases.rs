#[cfg(test)]
mod max_aliases_e2e_tests {
    use crate::testkit::{ClientResponseExt, TestRouterBuilder, TestSubgraphsBuilder};

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

        insta::assert_snapshot!(res.json_body_string_pretty().await, @r###"
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

        insta::assert_snapshot!(res.json_body_string_pretty().await, @r###"
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
