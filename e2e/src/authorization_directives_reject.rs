#[cfg(test)]
mod authorization_directives_in_reject_mode_e2e_tests {
    use jsonwebtoken::{encode, EncodingKey};
    use sonic_rs::{json, Value};
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::testkit::{some_header_map, ClientResponseExt, TestRouterBuilder, TestSubgraphsBuilder};

    fn generate_jwt(payload: &Value) -> String {
        let pem = include_str!("../jwks.rsa512.pem");

        encode::<Value>(
            &jsonwebtoken::Header {
                alg: jsonwebtoken::Algorithm::RS512,
                kid: Some("test_id".to_string()),
                ..Default::default()
            },
            payload,
            &EncodingKey::from_rsa_pem(pem.as_bytes()).expect("failed to read pem"),
        )
        .expect("failed to create token")
    }

    fn authorization_header_with_scopes(scopes: &str) -> http::HeaderMap {
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
                    "scope": scopes,
                }))
            )
        }
        .unwrap()
    }

    /// Verifies that an unauthenticated request to a field protected by `@authenticated`
    /// results in a 403 Forbidden response.
    #[ntex::test]
    async fn unauthenticated_access_to_authenticated_field() {
        let subgraphs = TestSubgraphsBuilder::new().build().start().await;
        let router = TestRouterBuilder::new()
            .with_subgraphs(&subgraphs)
            .file_config("configs/jwt_auth.directives.reject.router.yaml")
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(
                "{ me { id name } topProducts(first: 1) { upc } }",
                None,
                None,
            )
            .await;
        assert_eq!(res.status(), 403, "Expected 403 Forbidden");

        let subgraph_requests = subgraphs.get_requests_log("accounts");
        assert_eq!(
            subgraph_requests.map(|r| r.len()).unwrap_or(0),
            0,
            "expected 0 requests to accounts subgraph"
        );

        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
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
        let subgraphs = TestSubgraphsBuilder::new().build().start().await;
        let router = TestRouterBuilder::new()
            .with_subgraphs(&subgraphs)
            .file_config("configs/jwt_auth.directives.reject.router.yaml")
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(
                "{ me { id name } }",
                None,
                Some(authorization_header_with_scopes("")),
            )
            .await;
        assert!(res.status().is_success(), "Expected 200 OK");

        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
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
        let subgraphs = TestSubgraphsBuilder::new().build().start().await;
        let router = TestRouterBuilder::new()
            .with_subgraphs(&subgraphs)
            .file_config("configs/jwt_auth.directives.reject.router.yaml")
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(
                "{ me { birthday } }",
                None,
                Some(authorization_header_with_scopes("read:birthday")),
            )
            .await;
        assert!(res.status().is_success(), "Expected 200 OK");

        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
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
        let subgraphs = TestSubgraphsBuilder::new().build().start().await;
        let router = TestRouterBuilder::new()
            .with_subgraphs(&subgraphs)
            .file_config("configs/jwt_auth.directives.reject.router.yaml")
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(
                "query { topProducts { name shippingEstimate reviews { body } } me { name birthday } }",
                None,
                None,
            )
            .await;
        assert_eq!(res.status().as_u16(), 403, "Expected 403 Forbidden");

        assert_eq!(
            subgraphs
                .get_requests_log("accounts")
                .map(|r| r.len())
                .unwrap_or(0),
            0
        );
        assert_eq!(
            subgraphs
                .get_requests_log("inventory")
                .map(|r| r.len())
                .unwrap_or(0),
            0
        );
        assert_eq!(
            subgraphs
                .get_requests_log("reviews")
                .map(|r| r.len())
                .unwrap_or(0),
            0
        );

        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
        {
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
                "affectedPath": "topProducts.shippingEstimate"
              }
            },
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

    /// Tests a complex query where the user is authenticated and has some, but not all,
    /// of the required scopes. Verifies the request is rejected.
    #[ntex::test]
    async fn complex_query_partially_authorized() {
        let subgraphs = TestSubgraphsBuilder::new().build().start().await;
        let router = TestRouterBuilder::new()
            .with_subgraphs(&subgraphs)
            .file_config("configs/jwt_auth.directives.reject.router.yaml")
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(
                "query { topProducts { name shippingEstimate reviews { body } } me { name birthday } }",
                None,
                Some(authorization_header_with_scopes("read:shipping")),
            )
            .await;
        assert_eq!(res.status().as_u16(), 403, "Expected 403 Forbidden");

        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
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
        let subgraphs = TestSubgraphsBuilder::new().build().start().await;
        let router = TestRouterBuilder::new()
            .with_subgraphs(&subgraphs)
            .file_config("configs/jwt_auth.directives.reject.router.yaml")
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(
                "query { topProducts { name shippingEstimate reviews { body } } me { name birthday } }",
                None,
                Some(authorization_header_with_scopes(
                    "read:shipping read:birthday read:body",
                )),
            )
            .await;
        assert!(res.status().is_success(), "Expected 200 OK");

        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
        {
          "data": {
            "topProducts": [
              {
                "name": "Table",
                "shippingEstimate": 50,
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
                "shippingEstimate": 0,
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
                "shippingEstimate": 10,
                "reviews": [
                  {
                    "body": "Ut enim ad minima veniam, quis nostrum exercitationem ullam corporis suscipit laboriosam, nisi ut aliquid ex ea commodi consequatur? Quis autem"
                  }
                ]
              },
              {
                "name": "Chair",
                "shippingEstimate": 50,
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
                "shippingEstimate": 0,
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
        let subgraphs = TestSubgraphsBuilder::new().build().start().await;
        let router = TestRouterBuilder::new()
            .with_subgraphs(&subgraphs)
            .file_config("configs/jwt_auth.directives.reject.router.yaml")
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(
                r#"
                  query($should_include: Boolean!) {
                    me {
                      id
                      birthday @include(if: $should_include)
                    }
                  }
                "#,
                Some(json!({ "should_include": false })),
                // User has a valid token but lacks the "read:birthday" scope.
                Some(authorization_header_with_scopes("read:user")),
            )
            .await;
        assert!(
            res.status().is_success(),
            "Expected 200 OK because the unauthorized field is not included"
        );

        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
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
        let subgraphs = TestSubgraphsBuilder::new().build().start().await;
        let router = TestRouterBuilder::new()
            .with_subgraphs(&subgraphs)
            .file_config("configs/jwt_auth.directives.reject.router.yaml")
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(
                "query($includeBirthday: Boolean!) { me { id birthday @include(if: $includeBirthday) } }",
                Some(json!({ "includeBirthday": true })),
                Some(authorization_header_with_scopes("")),
            )
            .await;
        assert_eq!(res.status().as_u16(), 403, "Expected 403 Forbidden");

        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
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
        let subgraphs = TestSubgraphsBuilder::new().build().start().await;
        let router = TestRouterBuilder::new()
            .with_subgraphs(&subgraphs)
            .file_config("configs/jwt_auth.directives.reject.router.yaml")
            .build()
            .start()
            .await;

        let query = "query { topProducts(first: 1) { name internal } }";

        // Test 1: Failure with only one of the required scopes
        let res = router
            .send_graphql_request(
                query,
                None,
                Some(authorization_header_with_scopes("read:internal")),
            )
            .await;
        assert_eq!(res.status().as_u16(), 403, "Expected 403 Forbidden");

        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
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
        let res = router
            .send_graphql_request(
                query,
                None,
                Some(authorization_header_with_scopes("read:internal admin")),
            )
            .await;
        assert!(res.status().is_success(), "Expected 200 OK");

        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
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
        let subgraphs = TestSubgraphsBuilder::new().build().start().await;
        let router = TestRouterBuilder::new()
            .with_subgraphs(&subgraphs)
            .file_config("configs/jwt_auth.directives.reject.router.yaml")
            .build()
            .start()
            .await;

        let query = "query { topProducts(first: 1) { name notes } }";

        // Test 1: Success with the first scope
        let res = router
            .send_graphql_request(
                query,
                None,
                Some(authorization_header_with_scopes("read:notes")),
            )
            .await;
        assert!(res.status().is_success(), "Expected 200 OK");

        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
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
        let res = router
            .send_graphql_request(query, None, Some(authorization_header_with_scopes("admin")))
            .await;
        assert!(res.status().is_success(), "Expected 200 OK");

        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
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
        let res = router
            .send_graphql_request(
                query,
                None,
                Some(authorization_header_with_scopes("read:shipping")),
            )
            .await;
        assert_eq!(res.status().as_u16(), 403, "Expected 403 Forbidden");

        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
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

    /// Test querying interface field directly requires authentication
    /// According to spec: interface field policy is AND of all implementing types
    /// SocialAccount.url requires @authenticated on both TwitterAccount and GitHubAccount
    #[ntex::test]
    async fn interface_field_authenticated_on_interface() {
        let subgraphs = TestSubgraphsBuilder::new().build().start().await;
        let router = TestRouterBuilder::new()
            .with_subgraphs(&subgraphs)
            .file_config("configs/jwt_auth.directives.reject.router.yaml")
            .build()
            .start()
            .await;

        // Unauthenticated - should reject entire request with 403
        let res = router
            .send_graphql_request("{ me { socialAccounts { url } } }", None, None)
            .await;
        assert_eq!(res.status().as_u16(), 403, "Expected 403 Forbidden");

        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
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

        // Authenticated - should succeed
        let res = router
            .send_graphql_request(
                "{ me { socialAccounts { url } } }",
                None,
                Some(authorization_header_with_scopes("")),
            )
            .await;
        assert!(res.status().is_success(), "Expected 200 OK");

        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
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
        let subgraphs = TestSubgraphsBuilder::new().build().start().await;
        let router = TestRouterBuilder::new()
            .with_subgraphs(&subgraphs)
            .file_config("configs/jwt_auth.directives.reject.router.yaml")
            .build()
            .start()
            .await;

        // No scopes - should reject
        let res = router
            .send_graphql_request(
                "{ me { socialAccounts { handle } } }",
                None,
                Some(authorization_header_with_scopes("")),
            )
            .await;
        assert_eq!(res.status().as_u16(), 403, "Expected 403 Forbidden");

        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
        {
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
        let res = router
            .send_graphql_request(
                "{ me { socialAccounts { handle } } }",
                None,
                Some(authorization_header_with_scopes("read:twitter_handle")),
            )
            .await;
        assert_eq!(res.status().as_u16(), 403, "Expected 403 Forbidden");

        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
        {
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
        let res = router
            .send_graphql_request(
                "{ me { socialAccounts { handle } } }",
                None,
                Some(authorization_header_with_scopes(
                    "read:twitter_handle read:github_handle",
                )),
            )
            .await;
        assert!(res.status().is_success(), "Expected 200 OK");

        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
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

    /// Test inline fragment on specific implementor
    /// According to spec section 4.3.2: When using inline fragments, authorization is
    /// applied based on the concrete type within the fragment
    /// Query: socialAccounts { ... on GitHubAccount { handle } }
    /// Expected: Only requires read:github_handle scope
    #[ntex::test]
    async fn interface_inline_fragment_github_only() {
        let subgraphs = TestSubgraphsBuilder::new().build().start().await;
        let router = TestRouterBuilder::new()
            .with_subgraphs(&subgraphs)
            .file_config("configs/jwt_auth.directives.reject.router.yaml")
            .build()
            .start()
            .await;

        // Only github scope - should succeed
        let res = router
            .send_graphql_request(
                "{ me { socialAccounts { ... on GitHubAccount { handle } } } }",
                None,
                Some(authorization_header_with_scopes("read:github_handle")),
            )
            .await;
        assert!(res.status().is_success(), "Expected 200 OK");

        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
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

        // No scope - should reject the entire request
        let res = router
            .send_graphql_request(
                "{ me { socialAccounts { ... on GitHubAccount { handle } } } }",
                None,
                Some(authorization_header_with_scopes("")),
            )
            .await;
        assert_eq!(res.status().as_u16(), 403, "Expected 403 Forbidden");

        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
        {
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
        let subgraphs = TestSubgraphsBuilder::new().build().start().await;
        let router = TestRouterBuilder::new()
            .with_subgraphs(&subgraphs)
            .file_config("configs/jwt_auth.directives.reject.router.yaml")
            .build()
            .start()
            .await;

        // Only twitter scope - should succeed
        let res = router
            .send_graphql_request(
                "{ me { socialAccounts { ... on TwitterAccount { handle } } } }",
                None,
                Some(authorization_header_with_scopes("read:twitter_handle")),
            )
            .await;
        assert!(res.status().is_success(), "Expected 200 OK");

        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
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

        // No scope - should reject the entire request
        let res = router
            .send_graphql_request(
                "{ me { socialAccounts { ... on TwitterAccount { handle } } } }",
                None,
                Some(authorization_header_with_scopes("")),
            )
            .await;
        assert_eq!(res.status().as_u16(), 403, "Expected 403 Forbidden");

        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
        {
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
        let subgraphs = TestSubgraphsBuilder::new().build().start().await;
        let router = TestRouterBuilder::new()
            .with_subgraphs(&subgraphs)
            .file_config("configs/jwt_auth.directives.reject.router.yaml")
            .build()
            .start()
            .await;

        // Only twitter scope - should reject (not partially succeed)
        let res = router
            .send_graphql_request(
                "{ me { socialAccounts { ... on GitHubAccount { handle } ... on TwitterAccount { handle } } } }",
                None,
                Some(authorization_header_with_scopes("read:twitter_handle")),
            )
            .await;
        assert_eq!(res.status().as_u16(), 403, "Expected 403 Forbidden");

        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
        {
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
        let res = router
            .send_graphql_request(
                "{ me { socialAccounts { ... on GitHubAccount { handle } ... on TwitterAccount { handle } } } }",
                None,
                Some(authorization_header_with_scopes("read:github_handle")),
            )
            .await;
        assert_eq!(res.status().as_u16(), 403, "Expected 403 Forbidden");

        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
        {
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
        let res = router
            .send_graphql_request(
                "{ me { socialAccounts { ... on GitHubAccount { handle } ... on TwitterAccount { handle } } } }",
                None,
                Some(authorization_header_with_scopes(
                    "read:twitter_handle read:github_handle",
                )),
            )
            .await;
        assert!(res.status().is_success(), "Expected 200 OK");

        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
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

    /// Test __typename and handle field authorization on interface
    /// Query: socialAccounts { __typename handle }
    /// Expected: handle field rejected without scopes, __typename shown with proper auth
    #[ntex::test]
    async fn interface_field_authorization_with_typename() {
        let subgraphs = TestSubgraphsBuilder::new().build().start().await;
        let router = TestRouterBuilder::new()
            .with_subgraphs(&subgraphs)
            .file_config("configs/jwt_auth.directives.reject.router.yaml")
            .build()
            .start()
            .await;

        // Authenticated with no scopes - should reject on handle field
        let res = router
            .send_graphql_request(
                "{ me { socialAccounts { __typename handle } } }",
                None,
                Some(authorization_header_with_scopes("")),
            )
            .await;
        assert_eq!(res.status().as_u16(), 403, "Expected 403 Forbidden");

        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
        {
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
        let res = router
            .send_graphql_request(
                "{ me { socialAccounts { __typename handle } } }",
                None,
                Some(authorization_header_with_scopes(
                    "read:twitter_handle read:github_handle",
                )),
            )
            .await;
        assert!(res.status().is_success(), "Expected 200 OK");

        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
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
