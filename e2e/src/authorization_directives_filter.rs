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

    /// Tests a complex query with multiple protected fields where the user is unauthenticated.
    /// It verifies that all protected fields are nulled and appropriate errors are returned,
    /// while public fields are still resolved.
    #[ntex::test]
    async fn complex_query_unauthenticated() {
        let subgraphs_server = SubgraphsServer::start().await;
        let app = init_router_from_config_file("configs/jwt_auth.directives.router.yaml")
            .await
            .unwrap();
        wait_for_readiness(&app.app).await;

        let req = init_graphql_request(
            "query { topProducts { name shippingEstimate reviews { body } } me { name birthday } }",
            None,
        );
        let resp = test::call_service(&app.app, req.to_request()).await;
        assert!(resp.status().is_success(), "Expected 200 OK");

        let body = test::read_body(resp).await;
        let json_body: Value = from_slice(&body).unwrap();

        let accounts_subgraph_requests = subgraphs_server
            .get_subgraph_requests_log("accounts")
            .await
            .unwrap_or_default();
        let inventory_subgraph_requests = subgraphs_server
            .get_subgraph_requests_log("inventory")
            .await
            .unwrap_or_default();
        let reviews_subgraph_requests = subgraphs_server
            .get_subgraph_requests_log("inventory")
            .await
            .unwrap_or_default();

        assert_eq!(
            accounts_subgraph_requests.len(),
            0,
            "expected 0 request to accounts subgraph as `me` is unauthorized"
        );
        assert_eq!(
            inventory_subgraph_requests.len(),
            0,
            "expected 0 request to inventory subgraph as `shippingEstimate` is unauthorized"
        );
        assert_eq!(
            reviews_subgraph_requests.len(),
            0,
            "expected 0 request to reviews subgraph as `body` is unauthorized"
        );

        insta::assert_snapshot!(to_string_pretty(&json_body).unwrap(), @r#"
        {
          "data": {
            "topProducts": [
              {
                "name": "Table",
                "shippingEstimate": null,
                "reviews": null
              },
              {
                "name": "Couch",
                "shippingEstimate": null,
                "reviews": null
              },
              {
                "name": "Glass",
                "shippingEstimate": null,
                "reviews": null
              },
              {
                "name": "Chair",
                "shippingEstimate": null,
                "reviews": null
              },
              {
                "name": "TV",
                "shippingEstimate": null,
                "reviews": null
              }
            ],
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
    /// of the required scopes. It verifies that only the fields for which the user is
    /// authorized are resolved.
    #[ntex::test]
    async fn complex_query_partially_authorized() {
        let _subgraphs_server = SubgraphsServer::start().await;
        let app = init_router_from_config_file("configs/jwt_auth.directives.router.yaml")
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

    /// Tests a complex query where the user is authenticated and has all required scopes.
    /// It verifies that the entire query is resolved successfully.
    #[ntex::test]
    async fn complex_query_fully_authorized() {
        let _subgraphs_server = SubgraphsServer::start().await;
        let app = init_router_from_config_file("configs/jwt_auth.directives.router.yaml")
            .await
            .unwrap();
        wait_for_readiness(&app.app).await;

        let req = init_graphql_request(
            "query { topProducts { name shippingEstimate reviews { body } } me { name birthday } }",
            None,
        )
        .header(
            header::AUTHORIZATION,
            authorization_header_value_with_scopes("read:shipping read:birthday"),
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

    /// Tests that a field protected by both `@authenticated` on its type and `@requiresScopes`
    /// on the field itself is inaccessible if the user is authenticated but lacks the required scope.
    #[ntex::test]
    async fn scope_and_condition_failure() {
        let _subgraphs_server = SubgraphsServer::start().await;
        let app = init_router_from_config_file("configs/jwt_auth.directives.router.yaml")
            .await
            .unwrap();
        wait_for_readiness(&app.app).await;

        let req = init_graphql_request(
            "query { topProducts { name notes internal shippingEstimate reviews { body } } me { name birthday } }",
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
                "name": "Table",
                "notes": "Notes for table",
                "internal": null,
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
                "notes": "Notes for couch",
                "internal": null,
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
                "notes": "Notes for glass",
                "internal": null,
                "shippingEstimate": null,
                "reviews": [
                  {
                    "body": "Ut enim ad minima veniam, quis nostrum exercitationem ullam corporis suscipit laboriosam, nisi ut aliquid ex ea commodi consequatur? Quis autem"
                  }
                ]
              },
              {
                "name": "Chair",
                "notes": "Notes for chair",
                "internal": null,
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
                "notes": "Notes for TV",
                "internal": null,
                "shippingEstimate": null,
                "reviews": []
              }
            ],
            "me": {
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
                "affectedPath": "topProducts.internal"
              }
            },
            {
              "message": "Unauthorized field or type",
              "extensions": {
                "code": "UNAUTHORIZED_FIELD_OR_TYPE",
                "affectedPath": "topProducts.shippingEstimate"
              }
            }
          ]
        }
        "#);
    }

    /// Verifies that a field is correctly included and resolved when `@include(if: true)`
    /// is used and the user is authorized.
    #[ntex::test]
    async fn include_authorized_field_with_true() {
        let _subgraphs_server = SubgraphsServer::start().await;
        let app = init_router_from_config_file("configs/jwt_auth.directives.router.yaml")
            .await
            .unwrap();
        wait_for_readiness(&app.app).await;

        let req = init_graphql_request(
        "query($includeBirthday: Boolean!) { me { id birthday @include(if: $includeBirthday) } }",
        Some(json!({ "includeBirthday": true })),
    )
    .header(
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
                "id": "1",
                "birthday": 1234567890
              }
            }
          }
        "#);
    }

    /// Verifies that an `@include(if: false)` directive correctly removes a field from the operation
    /// before authorization is checked. This ensures that no unnecessary auth logic is run on excluded fields.
    #[ntex::test]
    async fn include_authorized_field_with_false() {
        let _subgraphs_server = SubgraphsServer::start().await;
        let app = init_router_from_config_file("configs/jwt_auth.directives.router.yaml")
            .await
            .unwrap();
        wait_for_readiness(&app.app).await;

        let req = init_graphql_request(
          "query($includeBirthday: Boolean!) { me { id birthday @include(if: $includeBirthday) } }",
          Some(json!({ "includeBirthday": false })),
        )
        .header(
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
                "id": "1"
              }
            }
          }
        "#);
    }

    /// Verifies that no authorization error is returned for an unauthorized field
    /// when it is correctly excluded from the operation by `@include(if: false)`.
    /// This ensures users are not penalized for fields they did not request.
    #[ntex::test]
    async fn include_unauthorized_field_with_false() {
        let _subgraphs_server = SubgraphsServer::start().await;
        let app = init_router_from_config_file("configs/jwt_auth.directives.router.yaml")
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

    /// Verifies that a `@skip(if: true)` directive correctly removes a field from the operation
    /// before authorization is checked. This ensures that no unnecessary auth logic is run on excluded fields.
    #[ntex::test]
    async fn skip_authorized_field_with_true() {
        let _subgraphs_server = SubgraphsServer::start().await;
        let app = init_router_from_config_file("configs/jwt_auth.directives.router.yaml")
            .await
            .unwrap();
        wait_for_readiness(&app.app).await;

        let req = init_graphql_request(
            "query($skipBirthday: Boolean!) { me { id birthday @skip(if: $skipBirthday) } }",
            Some(json!({ "skipBirthday": true })),
        )
        .header(
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
                "id": "1"
              }
            }
          }
        "#);
    }

    /// Verifies that a field is correctly included and resolved when `@skip(if: false)`
    /// is used and the user is authorized.
    #[ntex::test]
    async fn skip_authorized_field_with_false() {
        let _subgraphs_server = SubgraphsServer::start().await;
        let app = init_router_from_config_file("configs/jwt_auth.directives.router.yaml")
            .await
            .unwrap();
        wait_for_readiness(&app.app).await;

        let req = init_graphql_request(
            "query($skipBirthday: Boolean!) { me { id birthday @skip(if: $skipBirthday) } }",
            Some(json!({ "skipBirthday": false })),
        )
        .header(
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
                "id": "1",
                "birthday": 1234567890
              }
            }
          }
        "#);
    }

    /// Verifies that authorization rules are still correctly enforced on a field that is conditionally
    /// included via `@include(if: true)`. An unauthorized user should be denied access and receive an error.
    #[ntex::test]
    async fn include_unauthorized_field_with_true() {
        let _subgraphs_server = SubgraphsServer::start().await;
        let app = init_router_from_config_file("configs/jwt_auth.directives.router.yaml")
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

    /// Tests the logical AND condition for `@requiresScopes`.
    /// A field with `scopes: [["scopeA", "scopeB"]]` requires both scopes to be present.
    #[ntex::test]
    async fn test_scope_and_logic() {
        let _subgraphs_server = SubgraphsServer::start().await;
        let app = init_router_from_config_file("configs/jwt_auth.directives.router.yaml")
            .await
            .unwrap();
        wait_for_readiness(&app.app).await;

        let query = "query { topProducts(first: 1) { name internal } }";

        // Test 1: Failure with only one of the required scopes
        let auth_header = authorization_header_value_with_scopes("read:internal");
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
        let app = init_router_from_config_file("configs/jwt_auth.directives.router.yaml")
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
        assert!(resp.status().is_success(), "Expected 200 OK");

        let body = test::read_body(resp).await;
        let json_body: Value = from_slice(&body).unwrap();

        insta::assert_snapshot!(to_string_pretty(&json_body).unwrap(), @r#"
          {
            "data": {
              "topProducts": [
                {
                  "name": "Table",
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

    #[ntex::test]
    async fn unauthorized_access_to_non_nullable_field_bubbles_up() {
        let _subgraphs_server = SubgraphsServer::start().await;
        let app = init_router_from_config_file("configs/jwt_auth.directives.router.yaml")
            .await
            .unwrap();
        wait_for_readiness(&app.app).await;

        let query = "query { topProducts(first: 1) { name price } }";

        let auth_header = authorization_header_value_with_scopes("read:shipping");
        let req = init_graphql_request(query, None).header(header::AUTHORIZATION, auth_header);
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
                  "affectedPath": "topProducts.price"
                }
              }
            ]
          }
        "#);
    }

    #[ntex::test]
    async fn interface_field_authenticated_on_interface() {
        let _subgraphs_server = SubgraphsServer::start().await;
        let app = init_router_from_config_file("configs/jwt_auth.directives.router.yaml")
            .await
            .unwrap();
        wait_for_readiness(&app.app).await;

        // Unauthenticated - should fail
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

    /// Test interface field with @requiresScopes on implementors
    /// Query: account { handle }
    /// Expected: Requires BOTH read:twitter_handle AND read:github_handle
    #[ntex::test]
    async fn interface_field_requires_all_implementor_scopes() {
        let _subgraphs_server = SubgraphsServer::start().await;
        let app = init_router_from_config_file("configs/jwt_auth.directives.router.yaml")
            .await
            .unwrap();
        wait_for_readiness(&app.app).await;

        // No scopes - should fail
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

        // Only twitter scope - should fail
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

    /// Test interface with both authenticated field and scoped field
    /// Query: account { url handle }
    /// Expected: Requires authentication AND both scopes
    #[ntex::test]
    async fn interface_field_authenticated_and_scoped() {
        let _subgraphs_server = SubgraphsServer::start().await;
        let app = init_router_from_config_file("configs/jwt_auth.directives.router.yaml")
            .await
            .unwrap();
        wait_for_readiness(&app.app).await;

        // Unauthenticated - should fail
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

        // Authenticated but no scopes - should fail on handle
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
    /// Query: account { ... on GitHubAccount { handle } }
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

        // No scope - should fail
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

    /// Test inline fragment on specific implementor
    /// Query: account { ... on TwitterAccount { handle } }
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
    }

    /// Test inline fragments on both implementors
    /// Query: account { ... on GitHubAccount { handle } ... on TwitterAccount { handle } }
    /// Expected: Requires BOTH scopes
    #[ntex::test]
    async fn interface_inline_fragments_both_implementors() {
        let _subgraphs_server = SubgraphsServer::start().await;
        let app = init_router_from_config_file("configs/jwt_auth.directives.router.yaml")
            .await
            .unwrap();
        wait_for_readiness(&app.app).await;

        // Only twitter scope - should partially succeed
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

        // Both scopes - should fully succeed
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
    /// Query: account { ... on GitHubAccount { url handle } }
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
    /// Query: account { ... on TwitterAccount { followers } }
    /// Expected: No auth required
    #[ntex::test]
    async fn interface_inline_fragment_unprotected_field() {
        let _subgraphs_server = SubgraphsServer::start().await;
        let app = init_router_from_config_file("configs/jwt_auth.directives.router.yaml")
            .await
            .unwrap();
        wait_for_readiness(&app.app).await;

        // Authenticated but no scopes - should succeed
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

    /// Test inline fragments with mixed authenticated fields
    /// Query: account { ... on GitHubAccount { url repoCount } ... on TwitterAccount { url followers } }
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

    /// Test __typename is always allowed
    /// Query: account { __typename handle }
    /// Expected: __typename works, handle requires both scopes
    #[ntex::test]
    async fn interface_typename_always_allowed() {
        let _subgraphs_server = SubgraphsServer::start().await;
        let app = init_router_from_config_file("configs/jwt_auth.directives.router.yaml")
            .await
            .unwrap();
        wait_for_readiness(&app.app).await;

        // Authenticated with no scopes - should get __typename but not handle
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
