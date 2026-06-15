#[cfg(test)]
mod tests {
    use e2e::testkit::{ClientResponseExt, TestRouter, TestSubgraphs};
    use hive_router::ntex;

    /// `on_graphql_validation` + `with_schema` swaps the schema used for
    /// validation, but `__type`/`__schema` introspection still resolves against
    /// the full supergraph today, so feature-gated fields remain visible through
    /// introspection even when they are blocked for queries.
    ///
    /// This test FAILS on stock hive-router main — see the assertion message for
    /// the actual leaked output.  The desired behaviour is for introspection to
    /// follow the same schema override, so that `with_schema` is the single knob
    /// that controls both validation and introspection consistently.
    #[ntex::test]
    async fn introspection_should_hide_fields_removed_by_validation_schema_override() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .file_config("../plugin_examples/feature_flags/router.config.yaml")
            .register_plugin::<crate::plugin::FeatureFlagsPlugin>()
            .build()
            .start()
            .await;

        // No x-feature-flags header: the plugin's `with_schema` override removes
        // both `inStock` and `shippingEstimate` from the validation schema.
        // Introspection must reflect the same filtered view.
        let res = router
            .send_graphql_request(
                r#"{ __type(name: "Product") { fields { name } } }"#,
                None,
                e2e::some_header_map! {},
            )
            .await;

        assert_eq!(res.status(), 200);
        let body = res.json_body_string_pretty().await;

        // Non-gated fields must still be visible.
        assert!(body.contains("\"name\": \"name\""), "non-gated field `name` missing: {body}");

        // Gated fields must not appear — if they do, introspection is leaking past
        // the validation-schema override applied by `with_schema`.
        assert!(
            !body.contains("shippingEstimate"),
            "`shippingEstimate` leaked through introspection even though the feature is \
             disabled.\nActual response:\n{body}"
        );
        assert!(
            !body.contains("inStock"),
            "`inStock` leaked through introspection even though the feature is disabled.\
             \nActual response:\n{body}"
        );
    }

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
