#[cfg(test)]
mod max_depth_e2e_tests {
    use ntex::web::test;
    use sonic_rs::{from_slice, to_string_pretty, Value};

    use crate::testkit::{
        init_graphql_request, init_router_from_config_inline, wait_for_readiness, SubgraphsServer,
    };
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
        let _subgraphs = SubgraphsServer::start().await;
        let app = init_router_from_config_inline(
            r#"
        limits:
            max_depth:
                n: 3
        "#,
        )
        .await
        .unwrap();
        wait_for_readiness(&app.app).await;
        let req = init_graphql_request(QUERY, None);

        let resp = test::call_service(&app.app, req.to_request()).await;
        let body = test::read_body(resp).await;
        let json_body: Value = from_slice(&body).unwrap();
        insta::assert_snapshot!(to_string_pretty(&json_body).unwrap(), @r###"
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
        let _subgraphs = SubgraphsServer::start().await;
        let app = init_router_from_config_inline(
            r#"
        limits:
            max_depth:
                n: 1
        "#,
        )
        .await
        .unwrap();
        wait_for_readiness(&app.app).await;

        let req = init_graphql_request(QUERY, None);
        let resp = test::call_service(&app.app, req.to_request()).await;
        let body = test::read_body(resp).await;
        let json_body: Value = from_slice(&body).unwrap();
        insta::assert_snapshot!(to_string_pretty(&json_body).unwrap(), @r###"
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
        let _subgraphs = SubgraphsServer::start().await;
        let app = init_router_from_config_inline(
            r#"
        limits:
            max_depth:
                n: 3
        "#,
        )
        .await
        .unwrap();
        wait_for_readiness(&app.app).await;
        let req = init_graphql_request(
            r#"
            query {
                me {
                    ...UnknownFragment
                }
            }
            "#,
            None,
        );

        let resp = test::call_service(&app.app, req.to_request()).await;
        let body = test::read_body(resp).await;
        let json_body: Value = from_slice(&body).unwrap();
        insta::assert_snapshot!(to_string_pretty(&json_body).unwrap(), @r###"
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
