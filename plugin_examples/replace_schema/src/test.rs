#[cfg(test)]
mod tests {
    use e2e::testkit::{ClientResponseExt, TestRouter, TestSubgraphs};
    use hive_router::ntex;

    #[ntex::test]
    async fn basic_variant_hides_feature_fields_from_validation_and_introspection() {
        let subgraphs = TestSubgraphs::builder().build().start().await;

        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .file_config("../plugin_examples/replace_schema/router.config.yaml")
            .register_plugin::<crate::plugin::ReplaceSchemaPlugin>()
            .build()
            .start()
            .await;

        // The default (no header) variant is the "full" schema: shippingEstimate is queryable.
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

        // With the "basic" variant, shippingEstimate has been stripped from the supergraph
        // before planning, so it's a validation error, same as if the field never existed.
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

        // Introspection respects the override too: shippingEstimate and inStock are entirely
        // gone from the "basic" variant's `Product` type, not just rejected by validation.
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

    /// A plugin whose selected document always fails `SchemaState` construction (a malformed
    /// subgraph endpoint URL), used to assert the router returns an internal error instead of
    /// silently falling back to the router's default schema.
    mod broken_schema_plugin {
        use std::sync::Arc;

        use hive_router::{
            async_trait,
            graphql_tools::static_graphql::schema::Document,
            plugins::{
                hooks::{
                    on_http_request::{OnHttpRequestHookPayload, OnHttpRequestHookResult},
                    on_plugin_init::{OnPluginInitPayload, OnPluginInitResult},
                },
                plugin_trait::{RouterPlugin, StartHookPayload},
            },
            query_planner::utils::parsing::safe_parse_schema,
        };

        const SUPERGRAPH_SDL: &str = include_str!("../supergraph.graphql");

        pub struct BrokenSchemaPlugin {
            document: Arc<Document>,
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
                let document = safe_parse_schema(&broken_sdl)?;
                payload.initialize_plugin(Self {
                    document: Arc::new(document),
                })
            }

            fn on_http_request<'req>(
                &'req self,
                payload: OnHttpRequestHookPayload<'req>,
            ) -> OnHttpRequestHookResult<'req> {
                payload.set_schema_document(self.document.clone());
                payload.proceed()
            }
        }
    }

    #[ntex::test]
    async fn failed_schema_state_build_returns_internal_error_not_default_schema() {
        let subgraphs = TestSubgraphs::builder().build().start().await;

        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                include_str!("../router.config.yaml").replace("replace_schema:", "broken_schema:"),
            )
            .register_plugin::<broken_schema_plugin::BrokenSchemaPlugin>()
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
                None,
            )
            .await;

        // Must not silently fall back to the router's default (working) schema: the plugin
        // selected a document whose `SchemaState` fails to build, so the request must fail.
        assert_eq!(res.status(), 500);
    }
}
