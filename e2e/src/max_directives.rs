#[cfg(test)]
mod max_directives_e2e_tests {
    use ntex::web::test;
    use sonic_rs::{from_slice, to_string_pretty, Value};

    use crate::testkit::{
        SubgraphsServer, init_graphql_request, init_router_from_config_inline, wait_for_readiness
    };

    #[ntex::test]
    async fn allows_query_within_max_directives() {
        let _subgraphs = SubgraphsServer::start().await;
        let app = init_router_from_config_inline(r#"
        limits:
            max_directives:
                n: 8
        "#)
            .await
            .unwrap();
        wait_for_readiness(&app.app).await;
        let req = init_graphql_request(
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
        );
        let resp = test::call_service(&app.app, req.to_request()).await;
        assert!(resp.status().is_success(), "Expected 200 OK");

        let body = test::read_body(resp).await;
        let json_body: Value = from_slice(&body).unwrap();
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
        let _subgraphs = SubgraphsServer::start().await;
        let app = init_router_from_config_inline(r#"
        limits:
            max_directives:
                n: 5
        "#)
            .await
            .unwrap();
        wait_for_readiness(&app.app).await;

        let req = init_graphql_request(
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
        );
        let resp = test::call_service(&app.app, req.to_request()).await;

        let body = test::read_body(resp).await;
        let json_body: Value = from_slice(&body).unwrap();

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
