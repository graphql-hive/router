#[cfg(test)]
mod disable_introspection_e2e_tests {
    use ntex::web::test;
    use sonic_rs::{from_slice, JsonContainerTrait, JsonValueTrait, Value};

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

        let errors = json_body.get("errors").unwrap().as_array().unwrap();
        assert_eq!(errors.len(), 1);
        let message = errors[0].get("message").unwrap().as_str().unwrap();
        assert_eq!(message, "Introspection queries are disabled.");
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

        let query_type_name = json_body["data"]["__schema"]["queryType"]["name"]
            .as_str()
            .expect("Expected query type name in response");
        assert_eq!(query_type_name, "Query");
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

        let errors = json_body.get("errors").unwrap().as_array().unwrap();
        assert_eq!(errors.len(), 1);
        let message = errors[0].get("message").unwrap().as_str().unwrap();
        assert_eq!(message, "Introspection queries are disabled.");
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
        let query_type_name = json_body["data"]["__schema"]["queryType"]["name"]
            .as_str()
            .unwrap();
        assert_eq!(query_type_name, "Query");
    }
}
