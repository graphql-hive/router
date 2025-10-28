#[cfg(test)]
mod authorization_directives_in_filter_mode_e2e_tests {
    use jsonwebtoken::{encode, EncodingKey};
    use ntex::http::header::{self, HeaderValue};
    use ntex::web::test;
    use sonic_rs::{from_slice, json, to_string_pretty, Value};
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::testkit::{
        init_graphql_request, init_router_from_config_file, wait_for_readiness, SubgraphsServer,
    };

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

    fn authorization_header_value_with_scopes(scopes: &str) -> HeaderValue {
        header::HeaderValue::from_str(&format!(
            "Bearer {}",
            generate_jwt(&json!({
                "sub": "user2",
                "iat": 1516239022,
                "exp": SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs()
                    + 3600,
                "scope": scopes,
            }))
        ))
        .unwrap()
    }

    /// Verifies that an unauthenticated request to a field protected by `@authenticated`
    /// results in an error and the field being nulled.
    #[ntex::test]
    async fn unauthenticated_access_to_authenticated_field() {
        let subgraphs_server = SubgraphsServer::start().await;
        let app = init_router_from_config_file("configs/jwt_auth.directives.router.yaml")
            .await
            .unwrap();
        wait_for_readiness(&app.app).await;

        let req = init_graphql_request("{ me { id name } topProducts(first: 1) { upc } }", None);
        let resp = test::call_service(&app.app, req.to_request()).await;
        assert!(resp.status().is_success(), "Expected 200 OK");

        let body = test::read_body(resp).await;
        let json_body: Value = from_slice(&body).unwrap();

        let subgraph_requests = subgraphs_server
            .get_subgraph_requests_log("accounts")
            .await
            .unwrap_or_default();
        assert_eq!(
            subgraph_requests.len(),
            0,
            "expected 0 requests to accounts subgraph"
        );

        insta::assert_snapshot!(to_string_pretty(&json_body).unwrap(), @r#"
        {
          "data": {
            "me": null,
            "topProducts": [
              {
                "upc": "1"
              }
            ]
          },
          "errors": [
            {
              "message": "Unauthorized field or type",
              "extensions": {
                "code": "UNAUTHORIZED_FIELD_OR_TYPE",
                "affectedPath": "me"
              }
            }
          ]
        }
        "#);
    }

    /// Verifies that an unauthenticated request to a field protected by `@authenticated`
    /// results in an error and the field being nulled.
    /// This test makes sure that filtered queries with empty selection sets are handled correctly.
    #[ntex::test]
    async fn unauthenticated_access_to_authenticated_field_empty_query() {
        let subgraphs_server = SubgraphsServer::start().await;
        let app = init_router_from_config_file("configs/jwt_auth.directives.router.yaml")
            .await
            .unwrap();
        wait_for_readiness(&app.app).await;

        let req = init_graphql_request("{ me { id } }", None);
        let resp = test::call_service(&app.app, req.to_request()).await;
        println!("{}", resp.status());
        assert!(resp.status().is_success(), "Expected 200 OK");

        let body = test::read_body(resp).await;
        let json_body: Value = from_slice(&body).unwrap();

        let subgraph_requests = subgraphs_server
            .get_subgraph_requests_log("accounts")
            .await
            .unwrap_or_default();
        assert_eq!(
            subgraph_requests.len(),
            0,
            "expected 0 requests to accounts subgraph"
        );

        insta::assert_snapshot!(to_string_pretty(&json_body).unwrap(), @r#"
        {
          "data": {
            "me": null
          },
          "errors": [
            {
              "message": "Unauthorized field or type",
              "extensions": {
                "code": "UNAUTHORIZED_FIELD_OR_TYPE",
                "affectedPath": "me"
              }
            }
          ]
        }
        "#);
    }

    /// Verifies that an authenticated request (with a valid JWT) to a field protected
    /// by `@authenticated` is successful.
    #[ntex::test]
    async fn authenticated_access_to_authenticated_field() {
        let subgraphs_server = SubgraphsServer::start().await;
        let app = init_router_from_config_file("configs/jwt_auth.directives.router.yaml")
            .await
            .unwrap();
        wait_for_readiness(&app.app).await;

        let req = init_graphql_request("{ me { id name } }", None).header(
            header::AUTHORIZATION,
            authorization_header_value_with_scopes(""),
        );
        let resp = test::call_service(&app.app, req.to_request()).await;
        assert!(resp.status().is_success(), "Expected 200 OK");

        let body = test::read_body(resp).await;
        let json_body: Value = from_slice(&body).unwrap();

        let subgraph_requests = subgraphs_server
            .get_subgraph_requests_log("accounts")
            .await
            .unwrap_or_default();
        assert_eq!(
            subgraph_requests.len(),
            1,
            "expected 1 request to accounts subgraph"
        );

        insta::assert_snapshot!(to_string_pretty(&json_body).unwrap(), @r#"
        {
          "data": {
            "me": {
              "id": "1",
              "name": "Uri Goldshtein"
            }
          }
        }
        "#);
    }

    /// Verifies that an authenticated request with the correct scope can access a field
    /// protected by `@requiresScopes`.
    #[ntex::test]
    async fn authenticated_access_to_scoped_field() {
        let subgraphs_server = SubgraphsServer::start().await;
        let app = init_router_from_config_file("configs/jwt_auth.directives.router.yaml")
            .await
            .unwrap();
        wait_for_readiness(&app.app).await;

        let req = init_graphql_request("{ me { birthday } }", None).header(
            header::AUTHORIZATION,
            authorization_header_value_with_scopes("read:birthday"),
        );
        let resp = test::call_service(&app.app, req.to_request()).await;
        assert!(resp.status().is_success(), "Expected 200 OK");

        let body = test::read_body(resp).await;
        let json_body: Value = from_slice(&body).unwrap();

        let subgraph_requests = subgraphs_server
            .get_subgraph_requests_log("accounts")
            .await
            .unwrap_or_default();
        assert_eq!(
            subgraph_requests.len(),
            1,
            "expected 1 request to accounts subgraph"
        );

        insta::assert_snapshot!(to_string_pretty(&json_body).unwrap(), @r#"
        {
          "data": {
            "me": {
              "birthday": 1234567890
            }
          }
        }
        "#);
    }

    /// Tests a complex query with multiple fields (some protected, some not) when the
    /// user is unauthenticated. Fields protected by `@authenticated` should be nulled.
    #[ntex::test]
    async fn complex_query_unauthenticated() {
        let _subgraphs_server = SubgraphsServer::start().await;
        let app = init_router_from_config_file("configs/jwt_auth.directives.router.yaml")
            .await
            .unwrap();
        wait_for_readiness(&app.app).await;

        let req = init_graphql_request(
            r#"
            query GetProductsAndMe {
                topProducts(first: 2) {
                    upc
                    name
                    price
                }
                me {
                    id
                    name
                    birthday
                }
            }
            "#,
            None,
        );
        let resp = test::call_service(&app.app, req.to_request()).await;
        assert!(resp.status().is_success(), "Expected 200 OK");

        let body = test::read_body(resp).await;
        let json_body: Value = from_slice(&body).unwrap();

        insta::assert_snapshot!(to_string_pretty(&json_body).unwrap(), @r#"
        {
          "data": {
            "topProducts": null,
            "me": null
          },
          "errors": [
            {
              "message": "Unauthorized field or type",
              "extensions": {
                "code": "UNAUTHORIZED_FIELD_OR_TYPE",
                "affectedPath": "me"
              }
            },
            {
              "message": "Unauthorized field or type",
              "extensions": {
                "code": "UNAUTHORIZED_FIELD_OR_TYPE",
                "affectedPath": "topProducts.price"
              }
            }
          ]
        }
        "#);
    }

    /// Tests a complex query with an authenticated user but missing some required scopes.
    /// Fields with unmet scope requirements should be nulled.
    #[ntex::test]
    async fn complex_query_partially_authorized() {
        let _subgraphs_server = SubgraphsServer::start().await;
        let app = init_router_from_config_file("configs/jwt_auth.directives.router.yaml")
            .await
            .unwrap();
        wait_for_readiness(&app.app).await;

        let req = init_graphql_request(
            r#"
            query GetProductsAndMe {
                topProducts(first: 2) {
                    upc
                    name
                    price
                    notes
                }
                me {
                    id
                    name
                    birthday
                }
            }
            "#,
            None,
        )
        .header(
            header::AUTHORIZATION,
            authorization_header_value_with_scopes("read:price"),
        );
        let resp = test::call_service(&app.app, req.to_request()).await;
        assert!(resp.status().is_success(), "Expected 200 OK");

        let body = test::read_body(resp).await;
        let json_body: Value = from_slice(&body).unwrap();

        insta::assert_snapshot!(to_string_pretty(&json_body).unwrap(), @r#"
        {
          "data": {
            "topProducts": [
              {
                "upc": "1",
                "name": "Table",
                "price": 899,
                "notes": null
              },
              {
                "upc": "2",
                "name": "Couch",
                "price": 1299,
                "notes": null
              }
            ],
            "me": {
              "id": "1",
              "name": "Uri Goldshtein",
              "birthday": null
            }
          },
          "errors": [
            {
              "message": "Unauthorized field or type",
              "extensions": {
                "code": "UNAUTHORIZED_FIELD_OR_TYPE",
                "affectedPath": "me.birthday"
              }
            },
            {
              "message": "Unauthorized field or type",
              "extensions": {
                "code": "UNAUTHORIZED_FIELD_OR_TYPE",
                "affectedPath": "topProducts.notes"
              }
            }
          ]
        }
        "#);
    }

    /// Tests a complex query with all required scopes, ensuring everything is accessible.
    #[ntex::test]
    async fn complex_query_fully_authorized() {
        let _subgraphs_server = SubgraphsServer::start().await;
        let app = init_router_from_config_file("configs/jwt_auth.directives.router.yaml")
            .await
            .unwrap();
        wait_for_readiness(&app.app).await;

        let req = init_graphql_request(
            r#"
            query GetProductsAndMe {
                topProducts(first: 2) {
                    upc
                    name
                    price
                    notes
                }
                me {
                    id
                    name
                    birthday
                }
            }
            "#,
            None,
        )
        .header(
            header::AUTHORIZATION,
            authorization_header_value_with_scopes("read:price read:notes read:birthday"),
        );
        let resp = test::call_service(&app.app, req.to_request()).await;
        assert!(resp.status().is_success(), "Expected 200 OK");

        let body = test::read_body(resp).await;
        let json_body: Value = from_slice(&body).unwrap();

        insta::assert_snapshot!(to_string_pretty(&json_body).unwrap(), @r#"
        {
          "data": {
            "topProducts": [
              {
                "upc": "1",
                "name": "Table",
                "price": 899,
                "notes": "Notes for table"
              },
              {
                "upc": "2",
                "name": "Couch",
                "price": 1299,
                "notes": "Notes for couch"
              }
            ],
            "me": {
              "id": "1",
              "name": "Uri Goldshtein",
              "birthday": 1234567890
            }
          }
        }
        "#);
    }

    /// Tests @requiresScopes with AND logic (scopes in same inner array)
    /// The `internal` field requires BOTH "read:internal" AND "admin"
    #[ntex::test]
    async fn scope_and_condition_failure() {
        let _subgraphs_server = SubgraphsServer::start().await;
        let app = init_router_from_config_file("configs/jwt_auth.directives.router.yaml")
            .await
            .unwrap();
        wait_for_readiness(&app.app).await;

        // Has only "read:internal" but not "admin" - should fail
        let req = init_graphql_request(
            r#"
            query {
                topProducts(first: 1) {
                    upc
                    internal
                }
            }
            "#,
            None,
        )
        .header(
            header::AUTHORIZATION,
            authorization_header_value_with_scopes("read:internal"),
        );
        let resp = test::call_service(&app.app, req.to_request()).await;
        assert!(resp.status().is_success(), "Expected 200 OK");

        let body = test::read_body(resp).await;
        let json_body: Value = from_slice(&body).unwrap();

        insta::assert_snapshot!(to_string_pretty(&json_body).unwrap(), @r#"
        {
          "data": {
            "topProducts": [
              {
                "upc": "1",
                "internal": null
              }
            ]
          },
          "errors": [
            {
              "message": "Unauthorized field or type",
              "extensions": {
                "code": "UNAUTHORIZED_FIELD_OR_TYPE",
                "affectedPath": "topProducts.internal"
              }
            }
          ]
        }
        "#);

        // Has only "admin" but not "read:internal" - should fail
        let req = init_graphql_request(
            r#"
            query {
                topProducts(first: 1) {
                    upc
                    internal
                }
            }
            "#,
            None,
        )
        .header(
            header::AUTHORIZATION,
            authorization_header_value_with_scopes("admin"),
        );
        let resp = test::call_service(&app.app, req.to_request()).await;
        assert!(resp.status().is_success(), "Expected 200 OK");

        let body = test::read_body(resp).await;
        let json_body: Value = from_slice(&body).unwrap();

        insta::assert_snapshot!(to_string_pretty(&json_body).unwrap(), @r#"
        {
          "data": {
            "topProducts": [
              {
                "upc": "1",
                "internal": null
              }
            ]
          },
          "errors": [
            {
              "message": "Unauthorized field or type",
              "extensions": {
                "code": "UNAUTHORIZED_FIELD_OR_TYPE",
                "affectedPath": "topProducts.internal"
              }
            }
          ]
        }
        "#);

        // Has both "read:internal" AND "admin" - should succeed
        let req = init_graphql_request(
            r#"
            query {
                topProducts(first: 1) {
                    upc
                    internal
                }
            }
            "#,
            None,
        )
        .header(
            header::AUTHORIZATION,
            authorization_header_value_with_scopes("read:internal admin"),
        );
        let resp = test::call_service(&app.app, req.to_request()).await;
        assert!(resp.status().is_success(), "Expected 200 OK");

        let body = test::read_body(resp).await;
        let json_body: Value = from_slice(&body).unwrap();

        insta::assert_snapshot!(to_string_pretty(&json_body).unwrap(), @r#"
        {
          "data": {
            "topProducts": [
              {
                "upc": "1",
                "internal": "Internal for table"
              }
            ]
          }
        }
        "#);
    }

    /// Tests @include directive with authorized field
    #[ntex::test]
    async fn include_authorized_field_with_true() {
        let _subgraphs_server = SubgraphsServer::start().await;
        let app = init_router_from_config_file("configs/jwt_auth.directives.router.yaml")
            .await
            .unwrap();
        wait_for_readiness(&app.app).await;

        let req = init_graphql_request(
            "query($include: Boolean!) { me { id name @include(if: $include) } }",
            Some(json!({"include": true})),
        )
        .header(
            header::AUTHORIZATION,
            authorization_header_value_with_scopes(""),
        );
        let resp = test::call_service(&app.app, req.to_request()).await;
        assert!(resp.status().is_success(), "Expected 200 OK");

        let body = test::read_body(resp).await;
        let json_body: Value = from_slice(&body).unwrap();

        insta::assert_snapshot!(to_string_pretty(&json_body).unwrap(), @r#"
        {
          "data": {
            "me": {
              "id": "1",
              "name": "Uri Goldshtein"
            }
          }
        }
        "#);
    }

    /// Tests @include directive with authorized field set to false
    #[ntex::test]
    async fn include_authorized_field_with_false() {
        let _subgraphs_server = SubgraphsServer::start().await;
        let app = init_router_from_config_file("configs/jwt_auth.directives.router.yaml")
            .await
            .unwrap();
        wait_for_readiness(&app.app).await;

        let req = init_graphql_request(
            "query($include: Boolean!) { me { id name @include(if: $include) } }",
            Some(json!({"include": false})),
        )
        .header(
            header::AUTHORIZATION,
            authorization_header_value_with_scopes(""),
        );
        let resp = test::call_service(&app.app, req.to_request()).await;
        assert!(resp.status().is_success(), "Expected 200 OK");

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

    /// Tests @include directive with unauthorized field set to false - should not error
    #[ntex::test]
    async fn include_unauthorized_field_with_false() {
        let _subgraphs_server = SubgraphsServer::start().await;
        let app = init_router_from_config_file("configs/jwt_auth.directives.router.yaml")
            .await
            .unwrap();
        wait_for_readiness(&app.app).await;

        let req = init_graphql_request(
            "query($include: Boolean!) { me { id birthday @include(if: $include) } }",
            Some(json!({"include": false})),
        )
        .header(
            header::AUTHORIZATION,
            authorization_header_value_with_scopes(""),
        );
        let resp = test::call_service(&app.app, req.to_request()).await;
        assert!(resp.status().is_success(), "Expected 200 OK");

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

    /// Tests @skip directive with authorized field set to true
    #[ntex::test]
    async fn skip_authorized_field_with_true() {
        let _subgraphs_server = SubgraphsServer::start().await;
        let app = init_router_from_config_file("configs/jwt_auth.directives.router.yaml")
            .await
            .unwrap();
        wait_for_readiness(&app.app).await;

        let req = init_graphql_request(
            "query($skip: Boolean!) { me { id name @skip(if: $skip) } }",
            Some(json!({"skip": true})),
        )
        .header(
            header::AUTHORIZATION,
            authorization_header_value_with_scopes(""),
        );
        let resp = test::call_service(&app.app, req.to_request()).await;
        assert!(resp.status().is_success(), "Expected 200 OK");

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

    /// Tests @skip directive with authorized field set to false
    #[ntex::test]
    async fn skip_authorized_field_with_false() {
        let _subgraphs_server = SubgraphsServer::start().await;
        let app = init_router_from_config_file("configs/jwt_auth.directives.router.yaml")
            .await
            .unwrap();
        wait_for_readiness(&app.app).await;

        let req = init_graphql_request(
            "query($skip: Boolean!) { me { id name @skip(if: $skip) } }",
            Some(json!({"skip": false})),
        )
        .header(
            header::AUTHORIZATION,
            authorization_header_value_with_scopes(""),
        );
        let resp = test::call_service(&app.app, req.to_request()).await;
        assert!(resp.status().is_success(), "Expected 200 OK");

        let body = test::read_body(resp).await;
        let json_body: Value = from_slice(&body).unwrap();

        insta::assert_snapshot!(to_string_pretty(&json_body).unwrap(), @r#"
        {
          "data": {
            "me": {
              "id": "1",
              "name": "Uri Goldshtein"
            }
          }
        }
        "#);
    }

    /// Tests @include directive with unauthorized field set to true - should error
    #[ntex::test]
    async fn include_unauthorized_field_with_true() {
        let _subgraphs_server = SubgraphsServer::start().await;
        let app = init_router_from_config_file("configs/jwt_auth.directives.router.yaml")
            .await
            .unwrap();
        wait_for_readiness(&app.app).await;

        let req = init_graphql_request(
            "query($include: Boolean!) { me { id birthday @include(if: $include) } }",
            Some(json!({"include": true})),
        )
        .header(
            header::AUTHORIZATION,
            authorization_header_value_with_scopes(""),
        );
        let resp = test::call_service(&app.app, req.to_request()).await;
        assert!(resp.status().is_success(), "Expected 200 OK");

        let body = test::read_body(resp).await;
        let json_body: Value = from_slice(&body).unwrap();

        insta::assert_snapshot!(to_string_pretty(&json_body).unwrap(), @r#"
        {
          "data": {
            "me": {
              "id": "1",
              "birthday": null
            }
          },
          "errors": [
            {
              "message": "Unauthorized field or type",
              "extensions": {
                "code": "UNAUTHORIZED_FIELD_OR_TYPE",
                "affectedPath": "me.birthday"
              }
            }
          ]
        }
        "#);
    }

    /// Tests @requiresScopes with AND logic (scopes in same inner array)
    #[ntex::test]
    async fn test_scope_and_logic() {
        let _subgraphs_server = SubgraphsServer::start().await;
        let app = init_router_from_config_file("configs/jwt_auth.directives.router.yaml")
            .await
            .unwrap();
        wait_for_readiness(&app.app).await;

        // internal field requires: [["read:internal", "admin"]]
        // Must have BOTH scopes
        let req = init_graphql_request(
            r#"
            query {
                topProducts(first: 1) {
                    upc
                    internal
                }
            }
            "#,
            None,
        )
        .header(
            header::AUTHORIZATION,
            authorization_header_value_with_scopes("read:internal admin"),
        );
        let resp = test::call_service(&app.app, req.to_request()).await;
        assert!(resp.status().is_success(), "Expected 200 OK");

        let body = test::read_body(resp).await;
        let json_body: Value = from_slice(&body).unwrap();

        insta::assert_snapshot!(to_string_pretty(&json_body).unwrap(), @r#"
        {
          "data": {
            "topProducts": [
              {
                "upc": "1",
                "internal": "Internal for table"
              }
            ]
          }
        }
        "#);

        // With only one scope - should fail
        let req = init_graphql_request(
            r#"
            query {
                topProducts(first: 1) {
                    upc
                    internal
                }
            }
            "#,
            None,
        )
        .header(
            header::AUTHORIZATION,
            authorization_header_value_with_scopes("read:internal"),
        );
        let resp = test::call_service(&app.app, req.to_request()).await;
        assert!(resp.status().is_success(), "Expected 200 OK");

        let body = test::read_body(resp).await;
        let json_body: Value = from_slice(&body).unwrap();

        insta::assert_snapshot!(to_string_pretty(&json_body).unwrap(), @r#"
        {
          "data": {
            "topProducts": [
              {
                "upc": "1",
                "internal": null
              }
            ]
          },
          "errors": [
            {
              "message": "Unauthorized field or type",
              "extensions": {
                "code": "UNAUTHORIZED_FIELD_OR_TYPE",
                "affectedPath": "topProducts.internal"
              }
            }
          ]
        }
        "#);
    }

    /// Tests @requiresScopes with OR logic (scopes in different inner arrays)
    #[ntex::test]
    async fn test_scope_or_logic() {
        let _subgraphs_server = SubgraphsServer::start().await;
        let app = init_router_from_config_file("configs/jwt_auth.directives.router.yaml")
            .await
            .unwrap();
        wait_for_readiness(&app.app).await;

        // notes field requires: [["read:notes"], ["admin"]]
        // Need EITHER "read:notes" OR "admin"

        // With "read:notes" - should succeed
        let req = init_graphql_request(
            r#"
            query {
                topProducts(first: 1) {
                    upc
                    notes
                }
            }
            "#,
            None,
        )
        .header(
            header::AUTHORIZATION,
            authorization_header_value_with_scopes("read:notes"),
        );
        let resp = test::call_service(&app.app, req.to_request()).await;
        assert!(resp.status().is_success(), "Expected 200 OK");

        let body = test::read_body(resp).await;
        let json_body: Value = from_slice(&body).unwrap();

        insta::assert_snapshot!(to_string_pretty(&json_body).unwrap(), @r#"
        {
          "data": {
            "topProducts": [
              {
                "upc": "1",
                "notes": "Notes for table"
              }
            ]
          }
        }
        "#);

        // With "admin" - should also succeed
        let req = init_graphql_request(
            r#"
            query {
                topProducts(first: 1) {
                    upc
                    notes
                }
            }
            "#,
            None,
        )
        .header(
            header::AUTHORIZATION,
            authorization_header_value_with_scopes("admin"),
        );
        let resp = test::call_service(&app.app, req.to_request()).await;
        assert!(resp.status().is_success(), "Expected 200 OK");

        let body = test::read_body(resp).await;
        let json_body: Value = from_slice(&body).unwrap();

        insta::assert_snapshot!(to_string_pretty(&json_body).unwrap(), @r#"
        {
          "data": {
            "topProducts": [
              {
                "upc": "1",
                "notes": "Notes for table"
              }
            ]
          }
        }
        "#);

        // Without either scope - should fail
        let req = init_graphql_request(
            r#"
            query {
                topProducts(first: 1) {
                    upc
                    notes
                }
            }
            "#,
            None,
        )
        .header(
            header::AUTHORIZATION,
            authorization_header_value_with_scopes(""),
        );
        let resp = test::call_service(&app.app, req.to_request()).await;
        assert!(resp.status().is_success(), "Expected 200 OK");

        let body = test::read_body(resp).await;
        let json_body: Value = from_slice(&body).unwrap();

        insta::assert_snapshot!(to_string_pretty(&json_body).unwrap(), @r#"
        {
          "data": {
            "topProducts": [
              {
                "upc": "1",
                "notes": null
              }
            ]
          },
          "errors": [
            {
              "message": "Unauthorized field or type",
              "extensions": {
                "code": "UNAUTHORIZED_FIELD_OR_TYPE",
                "affectedPath": "topProducts.notes"
              }
            }
          ]
        }
        "#);
    }

    /// Tests that unauthorized access to a non-nullable field causes null to bubble up
    #[ntex::test]
    async fn unauthorized_access_to_non_nullable_field_bubbles_up() {
        let _subgraphs_server = SubgraphsServer::start().await;
        let app = init_router_from_config_file("configs/jwt_auth.directives.router.yaml")
            .await
            .unwrap();
        wait_for_readiness(&app.app).await;

        // Review.body is non-nullable and requires authentication
        let req = init_graphql_request(r#"{ topProducts(first: 1) { reviews { body } } }"#, None);
        let resp = test::call_service(&app.app, req.to_request()).await;
        assert!(resp.status().is_success(), "Expected 200 OK");

        let body = test::read_body(resp).await;
        let json_body: Value = from_slice(&body).unwrap();

        insta::assert_snapshot!(to_string_pretty(&json_body).unwrap(), @r#"
        {
          "data": {
            "topProducts": null
          },
          "errors": [
            {
              "message": "Unauthorized field or type",
              "extensions": {
                "code": "UNAUTHORIZED_FIELD_OR_TYPE",
                "affectedPath": "topProducts.reviews.body"
              }
            }
          ]
        }
        "#);
    }

    /// Test querying interface field directly requires authentication
    /// According to spec: interface field policy is AND of all implementing types
    /// SocialAccount.url requires @authenticated on both TwitterAccount and GitHubAccount
    #[ntex::test]
    async fn interface_field_authenticated_on_interface() {
        let _subgraphs_server = SubgraphsServer::start().await;
        let app = init_router_from_config_file("configs/jwt_auth.directives.router.yaml")
            .await
            .unwrap();
        wait_for_readiness(&app.app).await;

        // Unauthenticated - should reject entire query
        let req = init_graphql_request("{ me { socialAccounts { url } } }", None);
        let resp = test::call_service(&app.app, req.to_request()).await;
        assert!(resp.status().is_success(), "Expected 200 OK");

        let body = test::read_body(resp).await;
        let json_body: Value = from_slice(&body).unwrap();

        insta::assert_snapshot!(to_string_pretty(&json_body).unwrap(), @r#"
          {
            "data": {
              "me": null
            },
            "errors": [
              {
                "message": "Unauthorized field or type",
                "extensions": {
                  "code": "UNAUTHORIZED_FIELD_OR_TYPE",
                  "affectedPath": "me"
                }
              }
            ]
          }
        "#);

        // Authenticated - should succeed
        let req = init_graphql_request("{ me { socialAccounts { url } } }", None).header(
            header::AUTHORIZATION,
            authorization_header_value_with_scopes(""),
        );
        let resp = test::call_service(&app.app, req.to_request()).await;
        assert!(resp.status().is_success(), "Expected 200 OK");

        let body = test::read_body(resp).await;
        let json_body: Value = from_slice(&body).unwrap();

        insta::assert_snapshot!(to_string_pretty(&json_body).unwrap(), @r#"
          {
            "data": {
              "me": {
                "socialAccounts": [
                  {
                    "url": "https://twitter.com/urigo"
                  },
                  {
                    "url": "https://github.com/urigo"
                  }
                ]
              }
            }
          }
        "#);
    }

    /// Test querying interface field directly with @requiresScopes
    /// According to spec: interface field policy is AND of all implementing types
    /// SocialAccount.handle effective policy:
    ///   TwitterAccount.handle requires [["read:twitter_handle"]]
    ///   GitHubAccount.handle requires [["read:github_handle"]]
    ///   Combined (AND): [["read:twitter_handle", "read:github_handle"]]
    /// Query: socialAccounts { handle }
    /// Expected: Requires BOTH read:twitter_handle AND read:github_handle
    #[ntex::test]
    async fn interface_field_requires_all_implementor_scopes() {
        let _subgraphs_server = SubgraphsServer::start().await;
        let app = init_router_from_config_file("configs/jwt_auth.directives.router.yaml")
            .await
            .unwrap();
        wait_for_readiness(&app.app).await;

        // No scopes - should reject
        let req = init_graphql_request("{ me { socialAccounts { handle } } }", None).header(
            header::AUTHORIZATION,
            authorization_header_value_with_scopes(""),
        );
        let resp = test::call_service(&app.app, req.to_request()).await;
        assert!(resp.status().is_success(), "Expected 200 OK");

        let body = test::read_body(resp).await;
        let json_body: Value = from_slice(&body).unwrap();

        insta::assert_snapshot!(to_string_pretty(&json_body).unwrap(), @r#"
          {
            "data": {
              "me": null
            },
            "errors": [
              {
                "message": "Unauthorized field or type",
                "extensions": {
                  "code": "UNAUTHORIZED_FIELD_OR_TYPE",
                  "affectedPath": "me.socialAccounts.handle"
                }
              }
            ]
          }
        "#);

        // Only twitter scope - should reject
        let req = init_graphql_request("{ me { socialAccounts { handle } } }", None).header(
            header::AUTHORIZATION,
            authorization_header_value_with_scopes("read:twitter_handle"),
        );
        let resp = test::call_service(&app.app, req.to_request()).await;
        assert!(resp.status().is_success(), "Expected 200 OK");

        let body = test::read_body(resp).await;
        let json_body: Value = from_slice(&body).unwrap();

        insta::assert_snapshot!(to_string_pretty(&json_body).unwrap(), @r#"
          {
            "data": {
              "me": null
            },
            "errors": [
              {
                "message": "Unauthorized field or type",
                "extensions": {
                  "code": "UNAUTHORIZED_FIELD_OR_TYPE",
                  "affectedPath": "me.socialAccounts.handle"
                }
              }
            ]
          }
        "#);

        // Both scopes - should succeed
        let req = init_graphql_request("{ me { socialAccounts { handle } } }", None).header(
            header::AUTHORIZATION,
            authorization_header_value_with_scopes("read:twitter_handle read:github_handle"),
        );
        let resp = test::call_service(&app.app, req.to_request()).await;
        assert!(resp.status().is_success(), "Expected 200 OK");

        let body = test::read_body(resp).await;
        let json_body: Value = from_slice(&body).unwrap();

        insta::assert_snapshot!(to_string_pretty(&json_body).unwrap(), @r#"
          {
            "data": {
              "me": {
                "socialAccounts": [
                  {
                    "handle": "@urigo"
                  },
                  {
                    "handle": "urigo"
                  }
                ]
              }
            }
          }
        "#);
    }

    /// Test querying multiple interface fields with different requirements
    /// Query: socialAccounts { url handle }
    /// Expected: Requires authentication AND both handle scopes
    #[ntex::test]
    async fn interface_field_authenticated_and_scoped() {
        let _subgraphs_server = SubgraphsServer::start().await;
        let app = init_router_from_config_file("configs/jwt_auth.directives.router.yaml")
            .await
            .unwrap();
        wait_for_readiness(&app.app).await;

        // Unauthenticated - should reject
        let req = init_graphql_request("{ me { socialAccounts { url handle } } }", None);
        let resp = test::call_service(&app.app, req.to_request()).await;
        assert!(resp.status().is_success(), "Expected 200 OK");

        let body = test::read_body(resp).await;
        let json_body: Value = from_slice(&body).unwrap();

        insta::assert_snapshot!(to_string_pretty(&json_body).unwrap(), @r#"
        {
          "data": {
            "me": null
          },
          "errors": [
            {
              "message": "Unauthorized field or type",
              "extensions": {
                "code": "UNAUTHORIZED_FIELD_OR_TYPE",
                "affectedPath": "me"
              }
            }
          ]
        }
        "#);

        // Authenticated but no scopes - should reject on handle
        let req = init_graphql_request("{ me { socialAccounts { url handle } } }", None).header(
            header::AUTHORIZATION,
            authorization_header_value_with_scopes(""),
        );
        let resp = test::call_service(&app.app, req.to_request()).await;
        assert!(resp.status().is_success(), "Expected 200 OK");

        let body = test::read_body(resp).await;
        let json_body: Value = from_slice(&body).unwrap();

        insta::assert_snapshot!(to_string_pretty(&json_body).unwrap(), @r#"
          {
            "data": {
              "me": null
            },
            "errors": [
              {
                "message": "Unauthorized field or type",
                "extensions": {
                  "code": "UNAUTHORIZED_FIELD_OR_TYPE",
                  "affectedPath": "me.socialAccounts.handle"
                }
              }
            ]
          }
        "#);

        // Authenticated with both scopes - should succeed
        let req = init_graphql_request("{ me { socialAccounts { url handle } } }", None).header(
            header::AUTHORIZATION,
            authorization_header_value_with_scopes("read:twitter_handle read:github_handle"),
        );
        let resp = test::call_service(&app.app, req.to_request()).await;
        assert!(resp.status().is_success(), "Expected 200 OK");

        let body = test::read_body(resp).await;
        let json_body: Value = from_slice(&body).unwrap();

        insta::assert_snapshot!(to_string_pretty(&json_body).unwrap(), @r#"
          {
            "data": {
              "me": {
                "socialAccounts": [
                  {
                    "url": "https://twitter.com/urigo",
                    "handle": "@urigo"
                  },
                  {
                    "url": "https://github.com/urigo",
                    "handle": "urigo"
                  }
                ]
              }
            }
          }
        "#);
    }

    /// Test inline fragment on specific implementor
    /// According to spec section 4.3.2: When using inline fragments, authorization is
    /// applied based on the concrete type within the fragment
    /// Query: socialAccounts { ... on GitHubAccount { handle } }
    /// Expected: Only requires read:github_handle scope
    #[ntex::test]
    async fn interface_inline_fragment_github_only() {
        let _subgraphs_server = SubgraphsServer::start().await;
        let app = init_router_from_config_file("configs/jwt_auth.directives.router.yaml")
            .await
            .unwrap();
        wait_for_readiness(&app.app).await;

        // Only github scope - should succeed
        let req = init_graphql_request(
            "{ me { socialAccounts { ... on GitHubAccount { handle } } } }",
            None,
        )
        .header(
            header::AUTHORIZATION,
            authorization_header_value_with_scopes("read:github_handle"),
        );
        let resp = test::call_service(&app.app, req.to_request()).await;
        assert!(resp.status().is_success(), "Expected 200 OK");

        let body = test::read_body(resp).await;
        let json_body: Value = from_slice(&body).unwrap();

        insta::assert_snapshot!(to_string_pretty(&json_body).unwrap(), @r#"
        {
          "data": {
            "me": {
              "socialAccounts": [
                {},
                {
                  "handle": "urigo"
                }
              ]
            }
          }
        }
        "#);

        // No scope - should reject the entire query
        let req = init_graphql_request(
            "{ me { socialAccounts { ... on GitHubAccount { handle } } } }",
            None,
        )
        .header(
            header::AUTHORIZATION,
            authorization_header_value_with_scopes(""),
        );
        let resp = test::call_service(&app.app, req.to_request()).await;
        assert!(resp.status().is_success(), "Expected 200 OK");

        let body = test::read_body(resp).await;
        let json_body: Value = from_slice(&body).unwrap();

        insta::assert_snapshot!(to_string_pretty(&json_body).unwrap(), @r#"
          {
            "data": {
              "me": null
            },
            "errors": [
              {
                "message": "Unauthorized field or type",
                "extensions": {
                  "code": "UNAUTHORIZED_FIELD_OR_TYPE",
                  "affectedPath": "me.socialAccounts.handle"
                }
              }
            ]
          }
        "#);
    }

    /// Test inline fragment on TwitterAccount
    /// Query: socialAccounts { ... on TwitterAccount { handle } }
    /// Expected: Only requires read:twitter_handle scope
    #[ntex::test]
    async fn interface_inline_fragment_twitter_only() {
        let _subgraphs_server = SubgraphsServer::start().await;
        let app = init_router_from_config_file("configs/jwt_auth.directives.router.yaml")
            .await
            .unwrap();
        wait_for_readiness(&app.app).await;

        // Only twitter scope - should succeed
        let req = init_graphql_request(
            "{ me { socialAccounts { ... on TwitterAccount { handle } } } }",
            None,
        )
        .header(
            header::AUTHORIZATION,
            authorization_header_value_with_scopes("read:twitter_handle"),
        );
        let resp = test::call_service(&app.app, req.to_request()).await;
        assert!(resp.status().is_success(), "Expected 200 OK");

        let body = test::read_body(resp).await;
        let json_body: Value = from_slice(&body).unwrap();

        insta::assert_snapshot!(to_string_pretty(&json_body).unwrap(), @r#"
          {
            "data": {
              "me": {
                "socialAccounts": [
                  {
                    "handle": "@urigo"
                  },
                  {}
                ]
              }
            }
          }
        "#);

        // No scope - should reject the entire query
        let req = init_graphql_request(
            "{ me { socialAccounts { ... on TwitterAccount { handle } } } }",
            None,
        )
        .header(
            header::AUTHORIZATION,
            authorization_header_value_with_scopes(""),
        );
        let resp = test::call_service(&app.app, req.to_request()).await;
        assert!(resp.status().is_success(), "Expected 200 OK");

        let body = test::read_body(resp).await;
        let json_body: Value = from_slice(&body).unwrap();

        insta::assert_snapshot!(to_string_pretty(&json_body).unwrap(), @r#"
          {
            "data": {
              "me": null
            },
            "errors": [
              {
                "message": "Unauthorized field or type",
                "extensions": {
                  "code": "UNAUTHORIZED_FIELD_OR_TYPE",
                  "affectedPath": "me.socialAccounts.handle"
                }
              }
            ]
          }
        "#);
    }

    /// Test inline fragments on BOTH implementors
    /// Query: socialAccounts { ... on GitHubAccount { handle } ... on TwitterAccount { handle } }
    /// According to spec: When querying with multiple fragments, all must be authorized
    /// Expected: Requires BOTH scopes (should reject if only partial access)
    #[ntex::test]
    async fn interface_inline_fragments_both_implementors() {
        let _subgraphs_server = SubgraphsServer::start().await;
        let app = init_router_from_config_file("configs/jwt_auth.directives.router.yaml")
            .await
            .unwrap();
        wait_for_readiness(&app.app).await;

        // Only twitter scope - should reject (not partially succeed)
        let req = init_graphql_request(
            "{ me { socialAccounts { ... on GitHubAccount { handle } ... on TwitterAccount { handle } } } }",
            None,
        )
        .header(
            header::AUTHORIZATION,
            authorization_header_value_with_scopes("read:twitter_handle"),
        );
        let resp = test::call_service(&app.app, req.to_request()).await;
        assert!(resp.status().is_success(), "Expected 200 OK");

        let body = test::read_body(resp).await;
        let json_body: Value = from_slice(&body).unwrap();

        insta::assert_snapshot!(to_string_pretty(&json_body).unwrap(), @r#"
          {
            "data": {
              "me": null
            },
            "errors": [
              {
                "message": "Unauthorized field or type",
                "extensions": {
                  "code": "UNAUTHORIZED_FIELD_OR_TYPE",
                  "affectedPath": "me.socialAccounts.handle"
                }
              }
            ]
          }
        "#);

        // Only github scope - should also reject
        let req = init_graphql_request(
            "{ me { socialAccounts { ... on GitHubAccount { handle } ... on TwitterAccount { handle } } } }",
            None,
        )
        .header(
            header::AUTHORIZATION,
            authorization_header_value_with_scopes("read:github_handle"),
        );
        let resp = test::call_service(&app.app, req.to_request()).await;
        assert!(resp.status().is_success(), "Expected 200 OK");

        let body = test::read_body(resp).await;
        let json_body: Value = from_slice(&body).unwrap();

        insta::assert_snapshot!(to_string_pretty(&json_body).unwrap(), @r#"
          {
            "data": {
              "me": null
            },
            "errors": [
              {
                "message": "Unauthorized field or type",
                "extensions": {
                  "code": "UNAUTHORIZED_FIELD_OR_TYPE",
                  "affectedPath": "me.socialAccounts.handle"
                }
              }
            ]
          }
        "#);

        // Both scopes - should succeed
        let req = init_graphql_request(
            "{ me { socialAccounts { ... on GitHubAccount { handle } ... on TwitterAccount { handle } } } }",
            None,
        )
        .header(
            header::AUTHORIZATION,
            authorization_header_value_with_scopes("read:twitter_handle read:github_handle"),
        );
        let resp = test::call_service(&app.app, req.to_request()).await;
        assert!(resp.status().is_success(), "Expected 200 OK");

        let body = test::read_body(resp).await;
        let json_body: Value = from_slice(&body).unwrap();

        insta::assert_snapshot!(to_string_pretty(&json_body).unwrap(), @r#"
          {
            "data": {
              "me": {
                "socialAccounts": [
                  {
                    "handle": "@urigo"
                  },
                  {
                    "handle": "urigo"
                  }
                ]
              }
            }
          }
        "#);
    }

    /// Test inline fragment with authenticated field
    /// Query: socialAccounts { ... on GitHubAccount { url handle } }
    /// Expected: Requires authentication and read:github_handle
    #[ntex::test]
    async fn interface_inline_fragment_authenticated_and_scoped() {
        let _subgraphs_server = SubgraphsServer::start().await;
        let app = init_router_from_config_file("configs/jwt_auth.directives.router.yaml")
            .await
            .unwrap();
        wait_for_readiness(&app.app).await;

        // Authenticated with github scope - should succeed
        let req = init_graphql_request(
            "{ me { socialAccounts { ... on GitHubAccount { url handle } } } }",
            None,
        )
        .header(
            header::AUTHORIZATION,
            authorization_header_value_with_scopes("read:github_handle"),
        );
        let resp = test::call_service(&app.app, req.to_request()).await;
        assert!(resp.status().is_success(), "Expected 200 OK");

        let body = test::read_body(resp).await;
        let json_body: Value = from_slice(&body).unwrap();

        insta::assert_snapshot!(to_string_pretty(&json_body).unwrap(), @r#"
          {
            "data": {
              "me": {
                "socialAccounts": [
                  {},
                  {
                    "url": "https://github.com/urigo",
                    "handle": "urigo"
                  }
                ]
              }
            }
          }
        "#);
    }

    /// Test inline fragment with unprotected field
    /// Query: socialAccounts { ... on TwitterAccount { followers } }
    /// Expected: No auth required (followers is not protected)
    #[ntex::test]
    async fn interface_inline_fragment_unprotected_field() {
        let _subgraphs_server = SubgraphsServer::start().await;
        let app = init_router_from_config_file("configs/jwt_auth.directives.router.yaml")
            .await
            .unwrap();
        wait_for_readiness(&app.app).await;

        // Authenticated but no scopes - should succeed (followers is unprotected)
        let req = init_graphql_request(
            "{ me { socialAccounts { ... on TwitterAccount { followers } } } }",
            None,
        )
        .header(
            header::AUTHORIZATION,
            authorization_header_value_with_scopes(""),
        );
        let resp = test::call_service(&app.app, req.to_request()).await;
        assert!(resp.status().is_success(), "Expected 200 OK");

        let body = test::read_body(resp).await;
        let json_body: Value = from_slice(&body).unwrap();

        insta::assert_snapshot!(to_string_pretty(&json_body).unwrap(), @r#"
          {
            "data": {
              "me": {
                "socialAccounts": [
                  {
                    "followers": 1000
                  },
                  {}
                ]
              }
            }
          }
        "#);
    }

    /// Test inline fragments with mixed authenticated fields (no scopes)
    /// Query: socialAccounts { ... on GitHubAccount { url repoCount } ... on TwitterAccount { url followers } }
    /// Expected: Requires authentication only (no scopes needed)
    #[ntex::test]
    async fn interface_inline_fragments_authenticated_no_scopes() {
        let _subgraphs_server = SubgraphsServer::start().await;
        let app = init_router_from_config_file("configs/jwt_auth.directives.router.yaml")
            .await
            .unwrap();
        wait_for_readiness(&app.app).await;

        // Authenticated without scopes - should succeed
        let req = init_graphql_request(
            "{ me { socialAccounts { ... on GitHubAccount { url repoCount } ... on TwitterAccount { url followers } } } }",
            None,
        )
        .header(
            header::AUTHORIZATION,
            authorization_header_value_with_scopes(""),
        );
        let resp = test::call_service(&app.app, req.to_request()).await;
        assert!(resp.status().is_success(), "Expected 200 OK");

        let body = test::read_body(resp).await;
        let json_body: Value = from_slice(&body).unwrap();

        insta::assert_snapshot!(to_string_pretty(&json_body).unwrap(), @r#"
          {
            "data": {
              "me": {
                "socialAccounts": [
                  {
                    "url": "https://twitter.com/urigo",
                    "followers": 1000
                  },
                  {
                    "url": "https://github.com/urigo",
                    "repoCount": 42
                  }
                ]
              }
            }
          }
        "#);
    }

    /// Test __typename and handle field authorization on interface
    /// Query: socialAccounts { __typename handle }
    /// Expected: handle field filtered without scopes, __typename shown with proper auth
    #[ntex::test]
    async fn interface_field_authorization_with_typename() {
        let _subgraphs_server = SubgraphsServer::start().await;
        let app = init_router_from_config_file("configs/jwt_auth.directives.router.yaml")
            .await
            .unwrap();
        wait_for_readiness(&app.app).await;

        // Authenticated with no scopes - should reject on handle field
        let req = init_graphql_request("{ me { socialAccounts { __typename handle } } }", None)
            .header(
                header::AUTHORIZATION,
                authorization_header_value_with_scopes(""),
            );
        let resp = test::call_service(&app.app, req.to_request()).await;
        assert!(resp.status().is_success(), "Expected 200 OK");

        let body = test::read_body(resp).await;
        let json_body: Value = from_slice(&body).unwrap();

        insta::assert_snapshot!(to_string_pretty(&json_body).unwrap(), @r#"
          {
            "data": {
              "me": null
            },
            "errors": [
              {
                "message": "Unauthorized field or type",
                "extensions": {
                  "code": "UNAUTHORIZED_FIELD_OR_TYPE",
                  "affectedPath": "me.socialAccounts.handle"
                }
              }
            ]
          }
        "#);

        // With both scopes - should get both fields
        let req = init_graphql_request("{ me { socialAccounts { __typename handle } } }", None)
            .header(
                header::AUTHORIZATION,
                authorization_header_value_with_scopes("read:twitter_handle read:github_handle"),
            );
        let resp = test::call_service(&app.app, req.to_request()).await;
        assert!(resp.status().is_success(), "Expected 200 OK");

        let body = test::read_body(resp).await;
        let json_body: Value = from_slice(&body).unwrap();

        insta::assert_snapshot!(to_string_pretty(&json_body).unwrap(), @r#"
          {
            "data": {
              "me": {
                "socialAccounts": [
                  {
                    "__typename": "TwitterAccount",
                    "handle": "@urigo"
                  },
                  {
                    "__typename": "GitHubAccount",
                    "handle": "urigo"
                  }
                ]
              }
            }
          }
        "#);
    }
}
