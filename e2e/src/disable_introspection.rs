#[cfg(test)]
mod disable_introspection_e2e_tests {
    use sonic_rs::{to_string_pretty, Value};

    use crate::testkit::{some_header_map, EnvVarsGuard, TestRouterBuilder, TestSubgraphsBuilder};

    #[ntex::test]
    async fn should_disable_based_on_env_var() {
        let _env_var_guard = EnvVarsGuard::new()
            .set("DISABLE_INTROSPECTION", "true")
            .apply()
            .await;

        let subgraphs = TestSubgraphsBuilder::new().build().start().await;
        let router = TestRouterBuilder::new()
            .with_subgraphs(&subgraphs)
            .file_config("configs/disable_introspection_env.yaml")
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request("{ __schema { queryType { name } } }", None, None)
            .await;

        let body = res.body().await.unwrap();
        let json_body: Value = sonic_rs::from_slice(&body).unwrap();

        insta::assert_snapshot!(to_string_pretty(&json_body).unwrap(), @r###"
        {
          "errors": [
            {
              "message": "Introspection queries are disabled",
              "extensions": {
                "code": "INTROSPECTION_DISABLED"
              }
            }
          ]
        }
        "###);
    }

    #[ntex::test]
    async fn should_enable_based_on_env_var() {
        let _env_var_guard = EnvVarsGuard::new()
            .set("DISABLE_INTROSPECTION", "false")
            .apply()
            .await;

        let subgraphs = TestSubgraphsBuilder::new().build().start().await;
        let router = TestRouterBuilder::new()
            .with_subgraphs(&subgraphs)
            .file_config("configs/disable_introspection_env.yaml")
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request("{ __schema { queryType { name } } }", None, None)
            .await;

        let body = res.body().await.unwrap();
        let json_body: Value = sonic_rs::from_slice(&body).unwrap();

        insta::assert_snapshot!(to_string_pretty(&json_body).unwrap(), @r#"
        {
          "data": {
            "__schema": {
              "queryType": {
                "name": "Query"
              }
            }
          }
        }
        "#);
    }

    #[ntex::test]
    async fn should_disable_based_on_headers() {
        let subgraphs = TestSubgraphsBuilder::new().build().start().await;
        let router = TestRouterBuilder::new()
            .with_subgraphs(&subgraphs)
            .file_config("configs/disable_introspection_header.yaml")
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(
                "{ __schema { queryType { name } } }",
                None,
                some_header_map! {
                    http::header::HeaderName::from_static("x-enable-introspection") => "false"
                },
            )
            .await;

        let body = res.body().await.unwrap();
        let json_body: Value = sonic_rs::from_slice(&body).unwrap();

        insta::assert_snapshot!(to_string_pretty(&json_body).unwrap(), @r###"
        {
          "errors": [
            {
              "message": "Introspection queries are disabled",
              "extensions": {
                "code": "INTROSPECTION_DISABLED"
              }
            }
          ]
        }
        "###);
    }

    #[ntex::test]
    async fn should_enable_based_on_headers() {
        let subgraphs = TestSubgraphsBuilder::new().build().start().await;
        let router = TestRouterBuilder::new()
            .with_subgraphs(&subgraphs)
            .file_config("configs/disable_introspection_header.yaml")
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(
                "{ __schema { queryType { name } } }",
                None,
                some_header_map! {
                    http::header::HeaderName::from_static("x-enable-introspection") => "true"
                },
            )
            .await;

        let body = res.body().await.unwrap();
        let json_body: Value = sonic_rs::from_slice(&body).unwrap();

        insta::assert_snapshot!(to_string_pretty(&json_body).unwrap(), @r#"
        {
          "data": {
            "__schema": {
              "queryType": {
                "name": "Query"
              }
            }
          }
        }
        "#);
    }
}
