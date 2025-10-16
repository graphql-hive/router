#[cfg(test)]
mod jwt_e2e_tests {
    use jsonwebtoken::{encode, EncodingKey};
    use ntex::http::header;
    use ntex::web::test;
    use sonic_rs::{from_slice, json, JsonValueTrait, Value};
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::testkit::{init_graphql_request, init_router_from_config_file, SubgraphsServer};

    fn generate_jwt(payload: &Value) -> String {
        let pem = include_str!("../jwks.rsa512.pem");

        encode::<Value>(
            &jsonwebtoken::Header {
                alg: jsonwebtoken::Algorithm::RS512,
                ..Default::default()
            },
            payload,
            &EncodingKey::from_rsa_pem(pem.as_bytes()).expect("failed to read pem"),
        )
        .expect("failed to create token")
    }

    #[ntex::test]
    async fn should_forward_claims_to_subgraph_via_extensions() {
        let subgraphs_server = SubgraphsServer::start().await;
        let app = init_router_from_config_file("configs/jwt_auth_forward.router.yaml")
            .await
            .unwrap();

        let exp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 3600;
        let claims = json!({
            "sub": "user1",
            "iat": 1516239022,
            "exp": exp,
        });
        let token = generate_jwt(&claims);

        let req = init_graphql_request("{ users { id } }", None).header(
            header::AUTHORIZATION,
            header::HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        );
        let resp = test::call_service(&app, req.to_request()).await;

        assert!(resp.status().is_success(), "Expected 200 OK");

        let subgraph_requests = subgraphs_server
            .get_subgraph_requests_log("accounts")
            .await
            .expect("expected requests sent to accounts subgraph");
        assert_eq!(
            subgraph_requests.len(),
            1,
            "expected 1 request to accounts subgraph"
        );

        let extensions = subgraph_requests[0].request_body.get("extensions").unwrap();

        assert_eq!(extensions.get("jwt").unwrap(), &claims);
    }

    #[ntex::test]
    async fn rejects_request_without_token_when_auth_is_required() {
        let app = init_router_from_config_file("configs/jwt_auth.router.yaml")
            .await
            .unwrap();

        let req = init_graphql_request("{ __typename }", None).to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(
            resp.status(),
            ntex::http::StatusCode::UNAUTHORIZED,
            "Expected 401 Unauthorized"
        );
        let body = test::read_body(resp).await;
        let json_body: Value = from_slice(&body).unwrap();

        assert_eq!(
            json_body["errors"][0]["message"],
            "jwt header lookup failed: failed to locate the value in the incoming request"
        );
        assert_eq!(
            json_body["errors"][0]["extensions"]["code"],
            "JWT_LOOKUP_FAILED"
        );
    }

    #[ntex::test]
    async fn rejects_request_with_malformed_token() {
        let app = init_router_from_config_file("configs/jwt_auth.router.yaml")
            .await
            .unwrap();

        let req = init_graphql_request("{ __typename }", None).header(
            header::AUTHORIZATION,
            header::HeaderValue::from_static("Bearer not-a-valid-jwt"),
        );

        let resp = test::call_service(&app, req.to_request()).await;

        assert_eq!(
            resp.status(),
            ntex::http::StatusCode::FORBIDDEN,
            "Expected 403 Forbidden"
        );
        let body = test::read_body(resp).await;
        let json_body: Value = from_slice(&body).unwrap();

        assert_eq!(
            json_body["errors"][0]["message"],
            "failed to parse JWT header: InvalidToken"
        );
        assert_eq!(
            json_body["errors"][0]["extensions"]["code"],
            "INVALID_JWT_HEADER"
        );
    }

    #[ntex::test]
    async fn rejects_request_with_invalid_signature() {
        let app = init_router_from_config_file("configs/jwt_auth.router.yaml")
            .await
            .unwrap();

        // This token is valid but signed with a different, unknown key.
        let token = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkpvaG4gRG9lIiwiaWF0IjoxNTE2MjM5MDIyfQ.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c";

        let req = init_graphql_request("{ __typename }", None).header(
            header::AUTHORIZATION,
            header::HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        );

        let resp = test::call_service(&app, req.to_request()).await;

        assert_eq!(
            resp.status(),
            ntex::http::StatusCode::FORBIDDEN,
            "Expected 403 Forbidden"
        );
    }

    #[ntex::test]
    async fn accepts_request_with_valid_token() {
        let app = init_router_from_config_file("configs/jwt_auth.router.yaml")
            .await
            .unwrap();

        let exp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 3600;
        let claims = json!({
            "sub": "user1",
            "iat": 1516239022,
            "exp": exp,
        });
        let token = generate_jwt(&claims);

        let req = init_graphql_request("{ __typename }", None).header(
            header::AUTHORIZATION,
            header::HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        );

        let resp = test::call_service(&app, req.to_request()).await;

        assert!(
            resp.status().is_success(),
            "Expected 2xx status for valid token"
        );
        let body = test::read_body(resp).await;
        let json_body: Value = from_slice(&body).unwrap();
        assert_eq!(json_body["data"]["__typename"], "Query");
    }

    #[ntex::test]
    async fn rejects_request_with_expired_token() {
        let app = init_router_from_config_file("configs/jwt_auth.router.yaml")
            .await
            .unwrap();

        let exp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            - 3600;
        let claims = json!({
            "sub": "user1",
            "iat": 1516239022,
            "exp": exp,
        });
        let token = generate_jwt(&claims);

        let req = init_graphql_request("{ __typename }", None).header(
            header::AUTHORIZATION,
            header::HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        );

        let resp = test::call_service(&app, req.to_request()).await;
        assert_eq!(
            resp.status(),
            ntex::http::StatusCode::FORBIDDEN,
            "Expected 403 for expired token"
        );
    }

    #[ntex::test]
    async fn rejects_request_with_wrong_issuer() {
        let app = init_router_from_config_file("configs/jwt_auth_issuer.router.yaml")
            .await
            .unwrap();

        let claims = json!({ "iss": "wrong-issuer", "exp": SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() + 3600 });
        let token = generate_jwt(&claims);

        let req = init_graphql_request("{ __typename }", None).header(
            header::AUTHORIZATION,
            header::HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        );

        let resp = test::call_service(&app, req.to_request()).await;
        assert_eq!(
            resp.status(),
            ntex::http::StatusCode::FORBIDDEN,
            "Expected 403 for wrong issuer"
        );
    }

    #[ntex::test]
    async fn rejects_request_with_wrong_audience() {
        let app = init_router_from_config_file("configs/jwt_auth_audience.router.yaml")
            .await
            .unwrap();

        let claims = json!({ "aud": "wrong-audience", "exp": SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() + 3600 });
        let token = generate_jwt(&claims);

        let req = init_graphql_request("{ __typename }", None).header(
            header::AUTHORIZATION,
            header::HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        );

        let resp = test::call_service(&app, req.to_request()).await;
        assert_eq!(
            resp.status(),
            ntex::http::StatusCode::FORBIDDEN,
            "Expected 403 for wrong audience"
        );
    }
}
