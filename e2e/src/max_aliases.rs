#[cfg(test)]
mod max_aliases_e2e_tests {
    use ntex::web::test;
    use sonic_rs::{from_slice, to_string_pretty, Value};

    use crate::testkit::{
        init_graphql_request, init_router_from_config_inline, wait_for_readiness, SubgraphsServer,
    };

    #[ntex::test]
    async fn allows_query_within_max_aliases() -> Result<(), Box<dyn std::error::Error>> {
        let _subgraphs = SubgraphsServer::start().await;
        let app = init_router_from_config_inline(
            r#"
        limits:
            max_aliases:
                n: 3
        "#,
        )
        .await?;
        wait_for_readiness(&app.app).await;
        let req = init_graphql_request(
            "query { 
                myInfo: me {
                    myName: name
                }
            }",
            None,
        );
        let resp = test::call_service(&app.app, req.to_request()).await;
        assert!(resp.status().is_success(), "Expected 200 OK");

        let body = test::read_body(resp).await;
        let json_body: Value = from_slice(&body)?;
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
        let _subgraphs = SubgraphsServer::start().await;
        let app = init_router_from_config_inline(
            r#"
        limits:
            max_aliases:
                n: 3
        "#,
        )
        .await?;
        wait_for_readiness(&app.app).await;

        let req = init_graphql_request(
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
        );
        let resp = test::call_service(&app.app, req.to_request()).await;

        let body = test::read_body(resp).await;
        let json_body: Value = from_slice(&body)?;

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
