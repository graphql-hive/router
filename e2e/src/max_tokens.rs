#[cfg(test)]
mod max_tokens_e2e_tests {
    use crate::testkit::{init_graphql_request, wait_for_readiness, SubgraphsServer};
    use ntex::web::test;
    use sonic_rs::{from_slice, to_string_pretty, Value};

    static QUERY: &str = r#"
        query {
            me {
                id
            }
        }
    "#;
    #[ntex::test]
    async fn does_not_reject_an_operation_below_token_limit() {
        let _subgraphs_server = SubgraphsServer::start().await;
        let app = crate::testkit::init_router_from_config_inline(
            r#"
            supergraph:
                source: file
                path: ./supergraph.graphql
            limits:
                max_tokens:
                    n: 100
            "#,
        )
        .await
        .unwrap();
        wait_for_readiness(&app.app).await;

        let req = init_graphql_request(QUERY, None);
        let resp = test::call_service(&app.app, req.to_request()).await;

        let body = test::read_body(resp).await;
        let json_body: Value = from_slice(&body).unwrap();
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
        let _subgraphs_server = SubgraphsServer::start().await;
        let app = crate::testkit::init_router_from_config_inline(
            r#"
            supergraph:
                source: file
                path: ./supergraph.graphql
            limits:
                max_tokens:
                    n: 4
            "#,
        )
        .await
        .unwrap();
        wait_for_readiness(&app.app).await;

        let req = init_graphql_request(QUERY, None);
        let resp = test::call_service(&app.app, req.to_request()).await;
        let body = test::read_body(resp).await;
        let json_body: Value = from_slice(&body).unwrap();

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
