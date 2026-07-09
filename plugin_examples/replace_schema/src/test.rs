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
}
