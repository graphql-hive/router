#[cfg(test)]
mod disable_introspection_e2e_tests {
    use crate::testkit::{
        some_header_map, ClientResponseExt, EnvVarsGuard, TestRouterBuilder, TestSubgraphsBuilder,
    };

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

        insta::assert_snapshot!(res.json_body_string_pretty().await, @r###"
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

        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
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

        insta::assert_snapshot!(res.json_body_string_pretty().await, @r###"
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

        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
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
