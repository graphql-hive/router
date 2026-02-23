#[cfg(test)]
mod jwt_e2e_tests {
    use jsonwebtoken::{encode, Algorithm, EncodingKey};
    use sonic_rs::{json, JsonValueTrait, Value};
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::testkit_v2::{some_header_map, TestRouterBuilder, TestSubgraphsBuilder};

    fn generate_jwt(payload: &Value) -> String {
        generate_jwt_with_alg(payload, jsonwebtoken::Algorithm::RS512)
    }

    fn generate_jwt_with_alg(payload: &Value, algorithm: Algorithm) -> String {
        let pem = include_str!("../jwks.rsa512.pem");

        encode::<Value>(
            &jsonwebtoken::Header {
                alg: algorithm,
                kid: Some("test_id".to_string()),
                ..Default::default()
            },
            payload,
            &EncodingKey::from_rsa_pem(pem.as_bytes()).expect("failed to read pem"),
        )
        .expect("failed to create token")
    }

    #[ntex::test]
    async fn should_forward_claims_to_subgraph_via_extensions() {
        let subgraphs = TestSubgraphsBuilder::new().build().start().await;
        let router = TestRouterBuilder::new()
            .with_subgraphs(&subgraphs)
            .file_config("configs/jwt_auth_forward.router.yaml")
            .build()
            .start()
            .await;

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

        let res = router
            .send_graphql_request(
                "{ users { id } }",
                None,
                some_header_map! {
                    http::header::AUTHORIZATION => format!("Bearer {}", token)
                },
            )
            .await;

        assert!(res.status().is_success(), "Expected 200 OK");

        let subgraph_requests = subgraphs
            .get_requests_log("accounts")
            .expect("expected requests sent to accounts subgraph");
        assert_eq!(
            subgraph_requests.len(),
            1,
            "expected 1 request to accounts subgraph"
        );

        let body: Value = sonic_rs::from_slice(
            subgraph_requests[0]
                .body
                .as_ref()
                .expect("expected request body"),
        )
        .expect("expected valid JSON body");
        let extensions = body.get("extensions").unwrap();

        assert_eq!(extensions.get("jwt").unwrap(), &claims);
    }

    #[ntex::test]
    async fn should_allow_expressions_to_access_jwt_details() {
        let subgraphs = TestSubgraphsBuilder::new().build().start().await;
        let router = TestRouterBuilder::new()
            .with_subgraphs(&subgraphs)
            .file_config("configs/jwt_auth_header_expression.router.yaml")
            .build()
            .start()
            .await;

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

        // First request with a valid token
        let res = router
            .send_graphql_request(
                "{ users { id } }",
                None,
                some_header_map! {
                    http::header::AUTHORIZATION => format!("Bearer {}", token)
                },
            )
            .await;
        assert!(res.status().is_success(), "Expected 200 OK");

        // Second request that is not authenticated at all
        let res = router
            .send_graphql_request("{ users { id } }", None, None)
            .await;
        assert!(res.status().is_success(), "Expected 200 OK");

        let subgraph_requests = subgraphs
            .get_requests_log("accounts")
            .expect("expected requests sent to accounts subgraph");
        assert_eq!(
            subgraph_requests.len(),
            2,
            "expected 2 request to accounts subgraph"
        );

        // First request should have the user id in the X-User-Id header
        let user_id_subgraph = subgraph_requests[0]
            .headers
            .get("x-user-id")
            .unwrap()
            .to_str()
            .unwrap();

        assert_eq!(user_id_subgraph, &claims["sub"]);

        // Second request should have "EMPTY"
        let user_id_subgraph = subgraph_requests[1]
            .headers
            .get("x-user-id")
            .unwrap()
            .to_str()
            .unwrap();

        assert_eq!(user_id_subgraph, "EMPTY");
    }

    #[ntex::test]
    async fn should_allow_expressions_to_access_jwt_scopes() {
        let subgraphs = TestSubgraphsBuilder::new().build().start().await;
        let router = TestRouterBuilder::new()
            .with_subgraphs(&subgraphs)
            .file_config("configs/jwt_auth_header_expression.router.yaml")
            .build()
            .start()
            .await;

        // First request with a token and "scope: read:accounts"
        let res = router
            .send_graphql_request(
                "{ users { id } }",
                None,
                some_header_map! {
                    http::header::AUTHORIZATION => format!(
                        "Bearer {}",
                        generate_jwt(&json!({
                            "sub": "user1",
                            "iat": 1516239022,
                            "exp": SystemTime::now()
                                .duration_since(UNIX_EPOCH)
                                .unwrap()
                                .as_secs()
                                + 3600,
                            "scope": "read:accounts write:accounts"
                        }))
                    )
                },
            )
            .await;
        assert!(res.status().is_success(), "Expected 200 OK");

        // Second request with other scopes
        let res = router
            .send_graphql_request(
                "{ users { id } }",
                None,
                some_header_map! {
                    http::header::AUTHORIZATION => format!(
                        "Bearer {}",
                        generate_jwt(&json!({
                            "sub": "user2",
                            "iat": 1516239022,
                            "exp": SystemTime::now()
                                .duration_since(UNIX_EPOCH)
                                .unwrap()
                                .as_secs()
                                + 3600,
                            "scope": "do:something_else"
                        }))
                    )
                },
            )
            .await;
        assert!(res.status().is_success(), "Expected 200 OK");

        // Third request with no scopes
        let res = router
            .send_graphql_request(
                "{ users { id } }",
                None,
                some_header_map! {
                    http::header::AUTHORIZATION => format!(
                        "Bearer {}",
                        generate_jwt(&json!({
                            "sub": "user3",
                            "iat": 1516239022,
                            "exp": SystemTime::now()
                                .duration_since(UNIX_EPOCH)
                                .unwrap()
                                .as_secs()
                                + 3600,
                        }))
                    )
                },
            )
            .await;
        assert!(res.status().is_success(), "Expected 200 OK");

        let subgraph_requests = subgraphs
            .get_requests_log("accounts")
            .expect("expected requests sent to accounts subgraph");
        assert_eq!(
            subgraph_requests.len(),
            3,
            "expected 3 request to accounts subgraph"
        );

        // First request should have the user id in the X-User-Id header
        assert_eq!(
            subgraph_requests[0]
                .headers
                .get("x-can-read")
                .unwrap()
                .to_str()
                .unwrap(),
            "Yes"
        );

        // Second and third requests should have "No"
        assert_eq!(
            subgraph_requests[1]
                .headers
                .get("x-can-read")
                .unwrap()
                .to_str()
                .unwrap(),
            "No"
        );
        assert_eq!(
            subgraph_requests[2]
                .headers
                .get("x-can-read")
                .unwrap()
                .to_str()
                .unwrap(),
            "No"
        );
    }

    #[ntex::test]
    async fn rejects_request_without_token_when_auth_is_required() {
        let router = TestRouterBuilder::new()
            .file_config("configs/jwt_auth.router.yaml")
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request("{ __typename }", None, None)
            .await;

        assert_eq!(
            res.status(),
            ntex::http::StatusCode::UNAUTHORIZED,
            "Expected 401 Unauthorized"
        );
        let body = res.body().await.unwrap();
        let json_body: Value = sonic_rs::from_slice(&body).unwrap();

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
        let router = TestRouterBuilder::new()
            .file_config("configs/jwt_auth.router.yaml")
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(
                "{ __typename }",
                None,
                some_header_map! {
                    http::header::AUTHORIZATION => "Bearer not-a-valid-jwt"
                },
            )
            .await;

        assert_eq!(
            res.status(),
            ntex::http::StatusCode::FORBIDDEN,
            "Expected 403 Forbidden"
        );
        let body = res.body().await.unwrap();
        let json_body: Value = sonic_rs::from_slice(&body).unwrap();

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
        let router = TestRouterBuilder::new()
            .file_config("configs/jwt_auth.router.yaml")
            .build()
            .start()
            .await;

        // This token is valid but signed with a different, unknown key.
        let token = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkpvaG4gRG9lIiwiaWF0IjoxNTE2MjM5MDIyfQ.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c";

        let res = router
            .send_graphql_request(
                "{ __typename }",
                None,
                some_header_map! {
                    http::header::AUTHORIZATION => format!("Bearer {}", token)
                },
            )
            .await;

        assert_eq!(
            res.status(),
            ntex::http::StatusCode::FORBIDDEN,
            "Expected 403 Forbidden"
        );
    }

    #[ntex::test]
    async fn accepts_request_with_valid_token() {
        let router = TestRouterBuilder::new()
            .file_config("configs/jwt_auth.router.yaml")
            .build()
            .start()
            .await;

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

        let res = router
            .send_graphql_request(
                "{ __typename }",
                None,
                some_header_map! {
                    http::header::AUTHORIZATION => format!("Bearer {}", token)
                },
            )
            .await;

        assert!(
            res.status().is_success(),
            "Expected 2xx status for valid token"
        );
        let body = res.body().await.unwrap();
        let json_body: Value = sonic_rs::from_slice(&body).unwrap();
        assert_eq!(json_body["data"]["__typename"], "Query");
    }

    #[ntex::test]
    async fn accepts_request_with_valid_token_jwk_with_alg() {
        let router = TestRouterBuilder::new()
            .file_config("configs/jwt_auth_jwk_with_alg.router.yaml")
            .build()
            .start()
            .await;

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

        let res = router
            .send_graphql_request(
                "{ __typename }",
                None,
                some_header_map! {
                    http::header::AUTHORIZATION => format!("Bearer {}", token)
                },
            )
            .await;

        assert!(
            res.status().is_success(),
            "Expected 2xx status for valid token with JWK that specifies alg"
        );
        let body = res.body().await.unwrap();
        let json_body: Value = sonic_rs::from_slice(&body).unwrap();
        assert_eq!(json_body["data"]["__typename"], "Query");
    }

    #[ntex::test]
    async fn rejects_request_with_expired_token() {
        let router = TestRouterBuilder::new()
            .file_config("configs/jwt_auth.router.yaml")
            .build()
            .start()
            .await;

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

        let res = router
            .send_graphql_request(
                "{ __typename }",
                None,
                some_header_map! {
                    http::header::AUTHORIZATION => format!("Bearer {}", token)
                },
            )
            .await;

        assert_eq!(
            res.status(),
            ntex::http::StatusCode::FORBIDDEN,
            "Expected 403 for expired token"
        );
    }

    #[ntex::test]
    async fn rejects_request_with_wrong_issuer() {
        let router = TestRouterBuilder::new()
            .file_config("configs/jwt_auth_issuer.router.yaml")
            .build()
            .start()
            .await;

        let claims = json!({
            "iss": "wrong-issuer",
            "exp": SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() + 3600,
        });
        let token = generate_jwt(&claims);

        let res = router
            .send_graphql_request(
                "{ __typename }",
                None,
                some_header_map! {
                    http::header::AUTHORIZATION => format!("Bearer {}", token)
                },
            )
            .await;

        assert_eq!(
            res.status(),
            ntex::http::StatusCode::FORBIDDEN,
            "Expected 403 for wrong issuer"
        );
    }

    #[ntex::test]
    async fn rejects_request_with_wrong_audience() {
        let router = TestRouterBuilder::new()
            .file_config("configs/jwt_auth_audience.router.yaml")
            .build()
            .start()
            .await;

        let claims = json!({
            "aud": "wrong-audience",
            "exp": SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() + 3600,
        });
        let token = generate_jwt(&claims);

        let res = router
            .send_graphql_request(
                "{ __typename }",
                None,
                some_header_map! {
                    http::header::AUTHORIZATION => format!("Bearer {}", token)
                },
            )
            .await;

        assert_eq!(
            res.status(),
            ntex::http::StatusCode::FORBIDDEN,
            "Expected 403 for wrong audience"
        );
    }

    #[ntex::test]
    async fn rejects_request_with_wrong_algorithm() {
        let router = TestRouterBuilder::new()
            .file_config("configs/jwt_auth_jwk_with_alg.router.yaml")
            .build()
            .start()
            .await;

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

        // Expects valid alg to be RS512 from config
        let token = generate_jwt_with_alg(&claims, jsonwebtoken::Algorithm::RS256);

        let res = router
            .send_graphql_request(
                "{ __typename }",
                None,
                some_header_map! {
                    http::header::AUTHORIZATION => format!("Bearer {}", token)
                },
            )
            .await;

        assert_eq!(
            res.status(),
            ntex::http::StatusCode::FORBIDDEN,
            "Expected 403 for wrong algorithm"
        );
    }
}
