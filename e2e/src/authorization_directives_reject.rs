#[cfg(test)]
mod authorization_directives_in_reject_mode_e2e_tests {
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
    /// results in a 403 Forbidden response.
    #[ntex::test]
    async fn unauthenticated_access_to_authenticated_field() {
        let subgraphs_server = SubgraphsServer::start().await;
        // This config file should have `unauthorized: { mode: "reject" }` set.
        let app = init_router_from_config_file("configs/jwt_auth.directives.reject.router.yaml")
            .await
            .unwrap();
        wait_for_readiness(&app.app).await;

        let req = init_graphql_request("{ me { id name } topProducts(first: 1) { upc } }", None);
        let resp = test::call_service(&app.app, req.to_request()).await;
        assert_eq!(resp.status().as_u16(), 403, "Expected 403 Forbidden");

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
        let _subgraphs_server = SubgraphsServer::start().await;
        let app = init_router_from_config_file("configs/jwt_auth.directives.reject.router.yaml")
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
        let _subgraphs_server = SubgraphsServer::start().await;
        let app = init_router_from_config_file("configs/jwt_auth.directives.reject.router.yaml")
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

    /// Tests a complex query with multiple protected fields where the user is unauthenticated.
    /// It verifies that the entire request is rejected with a 403 status.
    #[ntex::test]
    async fn complex_query_unauthenticated() {
        let subgraphs_server = SubgraphsServer::start().await;
        let app = init_router_from_config_file("configs/jwt_auth.directives.reject.router.yaml")
            .await
            .unwrap();
        wait_for_readiness(&app.app).await;

        let req = init_graphql_request(
            "query { topProducts { name shippingEstimate reviews { body } } me { name birthday } }",
            None,
        );
        let resp = test::call_service(&app.app, req.to_request()).await;
        assert_eq!(resp.status().as_u16(), 403, "Expected 403 Forbidden");

        let body = test::read_body(resp).await;
        let json_body: Value = from_slice(&body).unwrap();

        assert_eq!(
            subgraphs_server
                .get_subgraph_requests_log("accounts")
                .await
                .unwrap_or_default()
                .len(),
            0
        );
        assert_eq!(
            subgraphs_server
                .get_subgraph_requests_log("inventory")
                .await
                .unwrap_or_default()
                .len(),
            0
        );
        assert_eq!(
            subgraphs_server
                .get_subgraph_requests_log("reviews")
                .await
                .unwrap_or_default()
                .len(),
            0
        );

        insta::assert_snapshot!(to_string_pretty(&json_body).unwrap(), @r#"
        {
          "errors": [
            {
              "message": "Unauthorized field or type",
              "extensions": {
                "code": "UNAUTHORIZED_FIELD_OR_TYPE",
                "affectedPath": "topProducts.shippingEstimate"
              }
            },
            {
              "message": "Unauthorized field or type",
              "extensions": {
                "code": "UNAUTHORIZED_FIELD_OR_TYPE",
                "affectedPath": "topProducts.reviews.body"
              }
            },
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

    /// Tests a complex query where the user is authenticated and has some, but not all,
    /// of the required scopes. Verifies the request is rejected.
    #[ntex::test]
    async fn complex_query_partially_authorized() {
        let _subgraphs_server = SubgraphsServer::start().await;
        let app = init_router_from_config_file("configs/jwt_auth.directives.reject.router.yaml")
            .await
            .unwrap();
        wait_for_readiness(&app.app).await;

        let req = init_graphql_request(
            "query { topProducts { name shippingEstimate reviews { body } } me { name birthday } }",
            None,
        )
        .header(
            header::AUTHORIZATION,
            authorization_header_value_with_scopes("read:shipping"),
        );
        let resp = test::call_service(&app.app, req.to_request()).await;
        assert_eq!(resp.status().as_u16(), 403, "Expected 403 Forbidden");

        let body = test::read_body(resp).await;
        let json_body: Value = from_slice(&body).unwrap();

        insta::assert_snapshot!(to_string_pretty(&json_body).unwrap(), @r#"
        {
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

    /// Tests a complex query where the user is authenticated and has all required scopes.
    /// It verifies that the entire query is resolved successfully.
    #[ntex::test]
    async fn complex_query_fully_authorized() {
        let _subgraphs_server = SubgraphsServer::start().await;
        let app = init_router_from_config_file("configs/jwt_auth.directives.reject.router.yaml")
            .await
            .unwrap();
        wait_for_readiness(&app.app).await;

        let req = init_graphql_request(
            "query { topProducts { name shippingEstimate reviews { body } } me { name birthday } }",
            None,
        )
        .header(
            header::AUTHORIZATION,
            authorization_header_value_with_scopes("read:shipping read:birthday read:body"),
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
                "name": "Table",
                "shippingEstimate": null,
                "reviews": [
                  {
                    "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum."
                  },
                  {
                    "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi"
                  },
                  {
                    "body": "sed quia consequuntur magni dolores eos qui ratione voluptatem sequi nesciunt. Neque porro quisquam est, qui dolorem ipsum quia dolor sit amet, consectetur, adipisci velit, sed quia non numquam eius modi tempora incidunt ut labore et dolore magnam aliquam quaerat voluptatem."
                  },
                  {
                    "body": "Ut enim ad minima veniam, quis nostrum exercitationem ullam corporis suscipit laboriosam, nisi ut aliquid ex ea commodi consequatur? Quis autem"
                  }
                ]
              },
              {
                "name": "Couch",
                "shippingEstimate": null,
                "reviews": [
                  {
                    "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum."
                  },
                  {
                    "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi"
                  },
                  {
                    "body": "sed quia consequuntur magni dolores eos qui ratione voluptatem sequi nesciunt. Neque porro quisquam est, qui dolorem ipsum quia dolor sit amet, consectetur, adipisci velit, sed quia non numquam eius modi tempora incidunt ut labore et dolore magnam aliquam quaerat voluptatem."
                  },
                  {
                    "body": "Ut enim ad minima veniam, quis nostrum exercitationem ullam corporis suscipit laboriosam, nisi ut aliquid ex ea commodi consequatur? Quis autem"
                  }
                ]
              },
              {
                "name": "Glass",
                "shippingEstimate": null,
                "reviews": [
                  {
                    "body": "Ut enim ad minima veniam, quis nostrum exercitationem ullam corporis suscipit laboriosam, nisi ut aliquid ex ea commodi consequatur? Quis autem"
                  }
                ]
              },
              {
                "name": "Chair",
                "shippingEstimate": null,
                "reviews": [
                  {
                    "body": "Ut enim ad minima veniam, quis nostrum exercitationem ullam corporis suscipit laboriosam, nisi ut aliquid ex ea commodi consequatur? Quis autem"
                  },
                  {
                    "body": "At vero eos et accusamus et iusto odio dignissimos ducimus qui blanditiis praesentium voluptatum deleniti atque corrupti quos dolores et quas molestias excepturi sint occaecati cupiditate non provident, similique sunt in culpa qui officia deserunt mollitia animi, id est laborum et dolorum fuga. Et harum quidem rerum facilis est et expedita distinctio. Nam libero tempore, cum soluta nobis est eligendi optio cumque nihil impedit quo minus id quod maxime placeat facere possimus, omnis voluptas assumenda est, omnis dolor repellendus. Temporibus autem quibusdam et aut officiis debitis aut rerum necessitatibus saepe eveniet ut et voluptates repudiandae sint et molestiae non recusandae. Itaque earum rerum hic tenetur a sapiente delectus, ut aut reiciendis voluptatibus maiores alias consequatur aut perferendis doloribus asperiores repellat."
                  }
                ]
              },
              {
                "name": "TV",
                "shippingEstimate": null,
                "reviews": []
              }
            ],
            "me": {
              "name": "Uri Goldshtein",
              "birthday": 1234567890
            }
          }
        }
        "#);
    }

    /// Verifies that no authorization error is returned for an unauthorized field
    /// when it is correctly excluded from the operation by `@include(if: false)`.
    #[ntex::test]
    async fn include_unauthorized_field_with_false() {
        let _subgraphs_server = SubgraphsServer::start().await;
        let app = init_router_from_config_file("configs/jwt_auth.directives.reject.router.yaml")
            .await
            .unwrap();
        wait_for_readiness(&app.app).await;

        let req = init_graphql_request(
            r#"
              query($should_include: Boolean!) {
                me {
                  id
                  birthday @include(if: $should_include)
                }
              }
            "#,
            Some(json!({ "should_include": false })),
        )
        .header(
            header::AUTHORIZATION,
            // User has a valid token but lacks the "read:birthday" scope.
            authorization_header_value_with_scopes("read:user"),
        );

        let resp = test::call_service(&app.app, req.to_request()).await;
        assert!(
            resp.status().is_success(),
            "Expected 200 OK because the unauthorized field is not included"
        );

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

    /// Verifies that authorization rules are enforced on a field that is conditionally
    /// included via `@include(if: true)`. An unauthorized user should be denied access.
    #[ntex::test]
    async fn include_unauthorized_field_with_true() {
        let _subgraphs_server = SubgraphsServer::start().await;
        let app = init_router_from_config_file("configs/jwt_auth.directives.reject.router.yaml")
            .await
            .unwrap();
        wait_for_readiness(&app.app).await;

        let req = init_graphql_request(
          "query($includeBirthday: Boolean!) { me { id birthday @include(if: $includeBirthday) } }",
          Some(json!({ "includeBirthday": true })),
        )
        .header(
            header::AUTHORIZATION,
            authorization_header_value_with_scopes(""),
        );
        let resp = test::call_service(&app.app, req.to_request()).await;
        assert_eq!(resp.status().as_u16(), 403, "Expected 403 Forbidden");

        let body = test::read_body(resp).await;
        let json_body: Value = from_slice(&body).unwrap();

        insta::assert_snapshot!(to_string_pretty(&json_body).unwrap(), @r#"
        {
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

    /// Tests the logical AND condition for `@requiresScopes`.
    /// A field with `scopes: [["scopeA", "scopeB"]]` requires both scopes to be present.
    #[ntex::test]
    async fn test_scope_and_logic() {
        let _subgraphs_server = SubgraphsServer::start().await;
        let app = init_router_from_config_file("configs/jwt_auth.directives.reject.router.yaml")
            .await
            .unwrap();
        wait_for_readiness(&app.app).await;

        let query = "query { topProducts(first: 1) { name internal } }";

        // Test 1: Failure with only one of the required scopes
        let auth_header = authorization_header_value_with_scopes("read:internal");
        let req = init_graphql_request(query, None).header(header::AUTHORIZATION, auth_header);
        let resp = test::call_service(&app.app, req.to_request()).await;
        assert_eq!(resp.status().as_u16(), 403, "Expected 403 Forbidden");

        let body = test::read_body(resp).await;
        let json_body: Value = from_slice(&body).unwrap();

        insta::assert_snapshot!(to_string_pretty(&json_body).unwrap(), @r#"
        {
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

        // Test 2: Success with both required scopes
        let auth_header = authorization_header_value_with_scopes("read:internal admin");
        let req = init_graphql_request(query, None).header(header::AUTHORIZATION, auth_header);
        let resp = test::call_service(&app.app, req.to_request()).await;
        assert!(resp.status().is_success(), "Expected 200 OK");

        let body = test::read_body(resp).await;
        let json_body: Value = from_slice(&body).unwrap();

        insta::assert_snapshot!(to_string_pretty(&json_body).unwrap(), @r#"
          {
            "data": {
              "topProducts": [
                {
                  "name": "Table",
                  "internal": "Internal for table"
                }
              ]
            }
          }
        "#);
    }

    /// Tests the logical OR condition for `@requiresScopes`.
    /// A field with `scopes: [["scopeA"], ["scopeB"]]` requires either scope to be present.
    #[ntex::test]
    async fn test_scope_or_logic() {
        let _subgraphs_server = SubgraphsServer::start().await;
        let app = init_router_from_config_file("configs/jwt_auth.directives.reject.router.yaml")
            .await
            .unwrap();
        wait_for_readiness(&app.app).await;

        let query = "query { topProducts(first: 1) { name notes } }";

        // Test 1: Success with the first scope
        let auth_header = authorization_header_value_with_scopes("read:notes");
        let req = init_graphql_request(query, None).header(header::AUTHORIZATION, auth_header);
        let resp = test::call_service(&app.app, req.to_request()).await;
        assert!(resp.status().is_success(), "Expected 200 OK");

        let body = test::read_body(resp).await;
        let json_body: Value = from_slice(&body).unwrap();

        insta::assert_snapshot!(to_string_pretty(&json_body).unwrap(), @r#"
        {
          "data": {
            "topProducts": [
              {
                "name": "Table",
                "notes": "Notes for table"
              }
            ]
          }
        }
        "#);

        // Test 2: Success with the second scope
        let auth_header = authorization_header_value_with_scopes("admin");
        let req = init_graphql_request(query, None).header(header::AUTHORIZATION, auth_header);
        let resp = test::call_service(&app.app, req.to_request()).await;
        assert!(resp.status().is_success(), "Expected 200 OK");

        let body = test::read_body(resp).await;
        let json_body: Value = from_slice(&body).unwrap();

        insta::assert_snapshot!(to_string_pretty(&json_body).unwrap(), @r#"
          {
            "data": {
              "topProducts": [
                {
                  "name": "Table",
                  "notes": "Notes for table"
                }
              ]
            }
          }
        "#);

        // Test 3: Failure with incorrect scope
        let auth_header = authorization_header_value_with_scopes("read:shipping");
        let req = init_graphql_request(query, None).header(header::AUTHORIZATION, auth_header);
        let resp = test::call_service(&app.app, req.to_request()).await;
        assert_eq!(resp.status().as_u16(), 403, "Expected 403 Forbidden");

        let body = test::read_body(resp).await;
        let json_body: Value = from_slice(&body).unwrap();

        insta::assert_snapshot!(to_string_pretty(&json_body).unwrap(), @r#"
        {
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
}
