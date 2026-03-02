#[cfg(test)]
mod tests {
    use e2e::testkit::{ClientResponseExt, TestRouter, TestSubgraphs};
    use hive_router::ntex;

    #[ntex::test]
    async fn do_not_allow_disabled_feature_flags() {
        let subgraphs = TestSubgraphs::builder().build().start().await;

        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
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
}
