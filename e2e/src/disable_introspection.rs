#[cfg(test)]
mod disable_introspection_e2e_tests {
    use ntex::web::test;
    use sonic_rs::{from_slice, to_string_pretty, Value};

    use crate::testkit::{
        init_graphql_request, init_router_from_config_file, wait_for_readiness, EnvVarGuard,
    };

    #[ntex::test]
    async fn should_disable_based_on_env_var() {
        let _env_var_guard = EnvVarGuard::new("DISABLE_INTROSPECTION", "true");

        let app = init_router_from_config_file("configs/disable_introspection_env.yaml")
            .await
            .unwrap();
        wait_for_readiness(&app.app).await;

        let req = init_graphql_request("{ __schema { queryType { name } } }", None);
        let resp = test::call_service(&app.app, req.to_request()).await;

        let body = test::read_body(resp).await;
        let json_body: Value = from_slice(&body).unwrap();

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
        let _env_var_guard = EnvVarGuard::new("DISABLE_INTROSPECTION", "false");

        let app = init_router_from_config_file("configs/disable_introspection_env.yaml")
            .await
            .unwrap();
        wait_for_readiness(&app.app).await;

        let req = init_graphql_request("{ __schema { queryType { name } } }", None);
        let resp = test::call_service(&app.app, req.to_request()).await;

        let body = test::read_body(resp).await;
        let json_body: Value = from_slice(&body).unwrap();

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
        let app = init_router_from_config_file("configs/disable_introspection_header.yaml")
            .await
            .unwrap();
        wait_for_readiness(&app.app).await;

        let req = init_graphql_request("{ __schema { queryType { name } } }", None)
            .header("X-Enable-Introspection", "false");
        let resp = test::call_service(&app.app, req.to_request()).await;

        let body = test::read_body(resp).await;
        let json_body: Value = from_slice(&body).unwrap();

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
        let app = init_router_from_config_file("configs/disable_introspection_header.yaml")
            .await
            .unwrap();
        wait_for_readiness(&app.app).await;

        let req = init_graphql_request("{ __schema { queryType { name } } }", None)
            .header("X-Enable-Introspection", "true");
        let resp = test::call_service(&app.app, req.to_request()).await;

        let body = test::read_body(resp).await;
        let json_body: Value = from_slice(&body).unwrap();
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
