#[cfg(test)]
mod tests {
    use e2e::testkit::{ClientResponseExt, EnvVarsGuard, TestRouter, TestSubgraphs};
    use hive_router::ntex;

    #[ntex::test]
    async fn do_not_allow_disabled_feature_flags() {
        let subgraphs = TestSubgraphs::builder().build().start().await;

        let _env_guard = EnvVarsGuard::new()
            .set("FEATURE_FLAGS_SUBGRAPHS_URL", &subgraphs.url())
            .apply()
            .await;

        let router = TestRouter::builder()
            .file_config("../plugin_examples/feature_flags/router.config.yaml")
            .register_plugin::<crate::plugin::FeatureFlagsPlugin>()
            .build()
            .start()
            .await;

        // shippingEstimate is not allowed
        let res = router
            .send_graphql_request(
                r#"
                query {
                    topProducts(first:1) {
                        name
                        price
                        inStock
                        shippingEstimate
                    }
                }
                "#,
                None,
                e2e::some_header_map! {
                    "x-feature-flags" => "inStock"
                },
            )
            .await;

        assert_eq!(res.status(), 400);

        e2e::insta::assert_snapshot!(res.json_body_string_pretty().await, @r###"
        {
          "errors": [
            {
              "message": "Cannot query field \"shippingEstimate\" on type \"Product\".",
              "locations": [
                {
                  "line": 7,
                  "column": 25
                }
              ],
              "extensions": {
                "code": "FieldsOnCorrectType"
              }
            }
          ]
        }
        "###);
    }

    #[ntex::test]
    async fn resolves_successfully_for_the_flags_enabled_in_the_header() {
        let subgraphs = TestSubgraphs::builder().build().start().await;

        let _env_guard = EnvVarsGuard::new()
            .set("FEATURE_FLAGS_SUBGRAPHS_URL", &subgraphs.url())
            .apply()
            .await;

        let router = TestRouter::builder()
            .file_config("../plugin_examples/feature_flags/router.config.yaml")
            .register_plugin::<crate::plugin::FeatureFlagsPlugin>()
            .build()
            .start()
            .await;

        // both feature-flagged fields are enabled: both resolve successfully
        let res = router
            .send_graphql_request(
                r#"
                query {
                    topProducts(first: 1) {
                        name
                        inStock
                        shippingEstimate
                    }
                }
                "#,
                None,
                e2e::some_header_map! {
                    "x-feature-flags" => "inStock,shippingEstimate"
                },
            )
            .await;

        assert_eq!(res.status(), 200);
        e2e::insta::assert_snapshot!(res.json_body_string_pretty().await, @r###"
        {
          "data": {
            "topProducts": [
              {
                "name": "Table",
                "inStock": true,
                "shippingEstimate": 50
              }
            ]
          }
        }
        "###);

        // only inStock is enabled: shippingEstimate is stripped from the schema, but the rest of
        // the query still resolves successfully
        let res = router
            .send_graphql_request(
                r#"
                query {
                    topProducts(first: 1) {
                        name
                        inStock
                    }
                }
                "#,
                None,
                e2e::some_header_map! {
                    "x-feature-flags" => "inStock"
                },
            )
            .await;

        assert_eq!(res.status(), 200);
        e2e::insta::assert_snapshot!(res.json_body_string_pretty().await, @r###"
        {
          "data": {
            "topProducts": [
              {
                "name": "Table",
                "inStock": true
              }
            ]
          }
        }
        "###);

        // no feature flags at all: both flagged fields are stripped, but the base query still
        // resolves successfully
        let res = router
            .send_graphql_request(
                r#"
                query {
                    topProducts(first: 1) {
                        name
                    }
                }
                "#,
                None,
                None,
            )
            .await;

        assert_eq!(res.status(), 200);
        e2e::insta::assert_snapshot!(res.json_body_string_pretty().await, @r###"
        {
          "data": {
            "topProducts": [
              {
                "name": "Table"
              }
            ]
          }
        }
        "###);
    }

    #[ntex::test]
    async fn failed_schema_state_build_returns_internal_error_not_default_schema() {
        let _env_guard = EnvVarsGuard::new()
            .set("FEATURE_FLAGS_SUBGRAPHS_URL", "not a valid url ::")
            .apply()
            .await;

        let router = TestRouter::builder()
            .file_config("../plugin_examples/feature_flags/router.config.yaml")
            .register_plugin::<crate::plugin::FeatureFlagsPlugin>()
            .skip_wait_for_ready_on_start()
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(
                r#"
                query {
                    topProducts(first: 1) {
                        name
                    }
                }
                "#,
                None,
                e2e::some_header_map! {
                    "x-feature-flags" => "inStock"
                },
            )
            .await;

        e2e::insta::assert_snapshot!(res.json_body_string_pretty().await, @r###"
        {
          "errors": [
            {
              "message": "Supergraph runtime error",
              "extensions": {
                "code": "SUPERGRAPH_RUNTIME_ERROR"
              }
            }
          ]
        }
        "###);
    }

    #[ntex::test]
    async fn none_selected_returns_error_no_supergraph_available() {
        let _env_guard = EnvVarsGuard::new()
            .set("FEATURE_FLAGS_SUBGRAPHS_URL", "doesnt matter")
            .apply()
            .await;

        let router = TestRouter::builder()
            .file_config("../plugin_examples/feature_flags/router.config.yaml")
            .register_plugin::<crate::plugin::FeatureFlagsPlugin>()
            .skip_wait_for_ready_on_start()
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(
                r#"
                query {
                    topProducts(first: 1) {
                        name
                    }
                }
                "#,
                None,
                e2e::some_header_map! {
                    "x-feature-flags" => "skip"
                },
            )
            .await;

        e2e::insta::assert_snapshot!(res.json_body_string_pretty().await, @r###"
        {
          "errors": [
            {
              "message": "No supergraph available yet, unable to process request",
              "extensions": {
                "code": "NO_SUPERGRAPH_AVAILABLE"
              }
            }
          ]
        }
        "###);
    }
}
