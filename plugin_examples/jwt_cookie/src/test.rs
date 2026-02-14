#[cfg(test)]
mod tests {
    use e2e::mockito::{self, ServerOpts};
    use e2e::testkit::{
        init_router_from_config_file_with_plugins, wait_for_readiness, SubgraphsServer,
    };
    use hive_router::ntex::web::test;
    use hive_router::{ntex, PluginRegistry};

    use crate::plugin::{NewTokenResult, JWT_TOKEN_NAME};

    #[ntex::test]
    async fn should_refresh_token_if_expired() {
        let mut refresh_endpoint_server = mockito::Server::new_with_opts_async(ServerOpts {
            port: 9876,
            ..Default::default()
        })
        .await;
        let existing_jwt_token = "existing_jwt_token";
        let existing_refresh_token = "existing_refresh_token";
        let existing_expired_at = "2000-01-01T00:00:00Z";
        let new_jwt_token = "new_jwt_token";
        let new_refresh_token = "new_refresh_token";
        let new_expired_at = "2100-01-01T00:00:00Z";
        let refresh_body = NewTokenResult {
            jwt_token: new_jwt_token.to_string(),
            refresh_token: new_refresh_token.to_string(),
            expired_at: new_expired_at.parse().unwrap(),
        };
        let refresh_mock = refresh_endpoint_server
            .mock("POST", "/")
            .match_body(mockito::Matcher::PartialJson(serde_json::json!({
                "refresh_token": existing_refresh_token
            })))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(serde_json::to_string(&refresh_body).unwrap())
            .create_async()
            .await;

        let subgraphs = SubgraphsServer::start().await;

        let app = init_router_from_config_file_with_plugins(
            "../plugin_examples/jwt_cookie/router.config.yaml",
            PluginRegistry::new().register::<crate::plugin::JwtCookiePlugin>(),
        )
        .await
        .expect("Router should initialize successfully");
        wait_for_readiness(&app.app).await;

        let resp = test::call_service(
            &app.app,
            test::TestRequest::post()
                .uri("/graphql")
                .set_payload(r#"{"query":"{ me { name } }"}"#)
                .header("content-type", "application/json")
                .header(
                    "cookie",
                    format!(
                        "{}={}; {}={}; {}={}",
                        JWT_TOKEN_NAME,
                        existing_jwt_token,
                        crate::plugin::JWT_REFRESH_TOKEN_NAME,
                        existing_refresh_token,
                        crate::plugin::JWT_EXPIRED_AT_NAME,
                        existing_expired_at
                    ),
                )
                .to_request(),
        )
        .await;

        // Check cookies
        let set_cookie_headers = resp.headers().get_all("set-cookie");
        let mut jwt_token_set = false;
        let mut refresh_token_set = false;
        let mut expired_at_set = false;
        for header_value in set_cookie_headers {
            let header_str = header_value.to_str().unwrap();
            if header_str.contains(JWT_TOKEN_NAME) {
                assert!(header_str.contains(&format!("{}={}", JWT_TOKEN_NAME, new_jwt_token)));
                jwt_token_set = true;
            } else if header_str.contains(crate::plugin::JWT_REFRESH_TOKEN_NAME) {
                assert!(header_str.contains(&format!(
                    "{}={}",
                    crate::plugin::JWT_REFRESH_TOKEN_NAME,
                    new_refresh_token
                )));
                refresh_token_set = true;
            } else if header_str.contains(crate::plugin::JWT_EXPIRED_AT_NAME) {
                let expected_header_str =
                    format!("{}={}", crate::plugin::JWT_EXPIRED_AT_NAME, new_expired_at);
                assert!(header_str.contains(&expected_header_str));
                expired_at_set = true;
            }
        }

        let body = test::read_body(resp).await;
        assert_eq!(body, r#"{"data":{"me":{"name":"Uri Goldshtein"}}}"#);

        assert!(
            jwt_token_set,
            "Expected jwt_token to be set in Set-Cookie header"
        );
        assert!(
            refresh_token_set,
            "Expected refresh_token to be set in Set-Cookie header"
        );
        assert!(
            expired_at_set,
            "Expected expired_at to be set in Set-Cookie header"
        );

        refresh_mock.assert_async().await;

        // Check if the subgraphs received the new JWT token in the headers of the request forwarded by the router
        let logs = subgraphs
            .get_subgraph_requests_log("accounts")
            .await
            .expect("Should be able to get subgraph requests log");
        let mut found_jwt_token_in_subgraph_request = false;
        for log in logs {
            println!("Subgraph request log: {:?}", log.headers);
            if let Some(auth_header) = log.headers.get("authorization") {
                if auth_header == &format!("Bearer {}", new_jwt_token) {
                    found_jwt_token_in_subgraph_request = true;
                    break;
                }
            }
        }
        assert!(found_jwt_token_in_subgraph_request, "Expected to find the new JWT token in the Authorization header of the request forwarded to the subgraph");
    }
}
