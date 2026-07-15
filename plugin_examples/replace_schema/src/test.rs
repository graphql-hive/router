#[cfg(test)]
mod tests {
    use e2e::testkit::{ClientResponseExt, EnvVarsGuard, TestRouter, TestSubgraphs};
    use hive_router::ntex;

    #[ntex::test]
    async fn basic_variant_hides_feature_fields_from_validation_and_introspection() {
        let subgraphs = TestSubgraphs::builder().build().start().await;

        let _env_guard = EnvVarsGuard::new()
            .set("REPLACE_SCHEMA_SUBGRAPHS_URL", &subgraphs.url())
            .apply()
            .await;

        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .file_config("../plugin_examples/replace_schema/router.config.yaml")
            .register_plugin::<crate::plugin::ReplaceSchemaPlugin>()
            .build()
            .start()
            .await;

        // default (no header) variant is the "full" schema: shippingEstimate is queryable
        let res = router
            .send_graphql_request(
                r#"
                query {
                    topProducts(first: 1) {
                        name
                        shippingEstimate
                    }
                }
                "#,
                None,
                None,
            )
            .await;
        assert_eq!(res.status(), 200);

        let res = router
            .send_graphql_request(
                r#"
                query {
                    topProducts(first: 1) {
                        name
                        shippingEstimate
                    }
                }
                "#,
                None,
                e2e::some_header_map! {
                    "x-schema-variant" => "basic"
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
                  "line": 5,
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

        // shippingEstimate and inStock are entirely gone from the "basic" variant's `Product`
        // type, not just rejected by validation
        let res = router
            .send_graphql_request(
                r#"
                query {
                    __type(name: "Product") {
                        fields {
                            name
                        }
                    }
                }
                "#,
                None,
                e2e::some_header_map! {
                    "x-schema-variant" => "basic"
                },
            )
            .await;
        assert_eq!(res.status(), 200);
        e2e::insta::assert_snapshot!(res.json_body_string_pretty().await, @r###"
        {
          "data": {
            "__type": {
              "fields": [
                {
                  "name": "upc"
                },
                {
                  "name": "weight"
                },
                {
                  "name": "price"
                },
                {
                  "name": "name"
                },
                {
                  "name": "reviews"
                },
                {
                  "name": "notes"
                },
                {
                  "name": "internal"
                }
              ]
            }
          }
        }
        "###);
    }

    /// a plugin whose override variant always fails `SchemaState` construction (a malformed
    /// subgraph endpoint URL), used to assert the router returns an error instead of
    /// silently falling back to the router's default schema when the override is selected
    mod broken_schema_plugin {
        use std::sync::Arc;

        use hive_router::{
            async_trait,
            plugins::{
                hooks::{
                    on_http_request::{OnHttpRequestHookPayload, OnHttpRequestHookResult},
                    on_plugin_init::{OnPluginInitPayload, OnPluginInitResult},
                    on_supergraph_load::SupergraphData,
                },
                plugin_trait::{RouterPlugin, StartHookPayload},
            },
        };

        const SUPERGRAPH_SDL: &str = include_str!("../supergraph.graphql");

        pub struct BrokenSchemaPlugin {
            broken_variant: Arc<SupergraphData>,
        }

        #[async_trait]
        impl RouterPlugin for BrokenSchemaPlugin {
            type Config = ();

            fn plugin_name() -> &'static str {
                "broken_schema"
            }

            fn on_plugin_init(payload: OnPluginInitPayload<Self>) -> OnPluginInitResult<Self> {
                let broken_sdl =
                    SUPERGRAPH_SDL.replace("http://0.0.0.0:4200/accounts", "not a valid url ::");
                let broken_variant = SupergraphData::from_sdl(&broken_sdl, Default::default())?;
                payload.initialize_plugin(Self {
                    broken_variant: Arc::new(broken_variant),
                })
            }

            fn on_http_request<'req>(
                &'req self,
                payload: OnHttpRequestHookPayload<'req>,
            ) -> OnHttpRequestHookResult<'req> {
                let variant = payload
                    .router_http_request
                    .headers()
                    .get("x-schema-variant")
                    .and_then(|value| value.to_str().ok());

                if variant == Some("basic") {
                    payload.set_supergraph(self.broken_variant.clone());
                }

                payload.proceed()
            }
        }
    }

    #[ntex::test]
    async fn failed_schema_state_build_returns_error_not_default_schema() {
        let subgraphs = TestSubgraphs::builder().build().start().await;

        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                // replace with broken_schema plugin
                include_str!("../router.config.yaml").replace("replace_schema:", "broken_schema:"),
            )
            .register_plugin::<broken_schema_plugin::BrokenSchemaPlugin>()
            .build()
            .start()
            .await;

        // No override header: the router's own default (working) schema is used,
        // unaffected by the plugin's broken variant
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

        // with the override header, the plugin selects its broken variant
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
                    "x-schema-variant" => "basic"
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
