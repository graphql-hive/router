#[cfg(test)]
mod tests {
    use e2e::testkit::{
        init_graphql_request, init_router_from_config_file_with_plugins, wait_for_readiness,
        SubgraphsServer,
    };
    use hive_router::ntex::web::test;
    use hive_router::PluginRegistry;
    use hive_router::{ntex, sonic_rs};

    #[ntex::test]
    async fn do_not_allow_disabled_feature_flags() {
        // shippingEstimate is not allowed
        let feature_flag_header = "inStock";
        let query = r#"
            query {
                topProducts(first:1) {
                    name
                    price
                    inStock
                    shippingEstimate
                }
            }
        "#;

        SubgraphsServer::start().await;

        let app = init_router_from_config_file_with_plugins(
            "../plugin_examples/feature_flags/router.config.yaml",
            PluginRegistry::new().register::<crate::plugin::FeatureFlagsPlugin>(),
        )
        .await
        .expect("Router should initialize successfully");

        wait_for_readiness(&app.app).await;

        let resp = test::call_service(
            &app.app,
            init_graphql_request(query, None)
                .header("x-feature-flags", feature_flag_header)
                .to_request(),
        )
        .await;

        assert!(resp.status().is_success(), "Expected 200 OK");

        let response_body = test::read_body(resp).await;

        let response_json = sonic_rs::from_slice::<sonic_rs::Value>(&response_body)
            .expect("Expected valid JSON response");
        let pretty_str = sonic_rs::to_string_pretty(&response_json)
            .expect("Failed to convert response JSON to pretty string");

        e2e::insta::assert_snapshot!(pretty_str, @r###"
        {
          "errors": [
            {
              "message": "Cannot query field \"shippingEstimate\" on type \"Product\".",
              "locations": [
                {
                  "line": 7,
                  "column": 21
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
}
