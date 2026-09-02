#[cfg(test)]
mod tests {
    use e2e::testkit::{ClientResponseExt, TestRouter, TestSubgraphs};
    use hive_router::ntex;

    /// No `x-user-role` header at all - `inStock` is denied and nulled, but
    /// `upc`/`name` (not policy-gated) still come back, and the subgraph
    /// still gets asked for whatever survived the filter.
    #[ntex::test]
    async fn denies_the_gated_field_without_the_admin_role() {
        let subgraphs = TestSubgraphs::builder().build().start().await;

        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .file_config("../plugin_examples/custom_policy/router.config.yaml")
            .register_plugin::<crate::plugin::CustomPolicyPlugin>()
            .build()
            .start()
            .await;

        let response = router
            .send_graphql_request("{ topProducts(first: 1) { upc name inStock } }", None, None)
            .await;

        e2e::insta::assert_snapshot!(response.json_body_string_pretty_stable().await, @r#"
        {
          "data": {
            "topProducts": [
              {
                "inStock": null,
                "name": "Table",
                "upc": "1"
              }
            ]
          },
          "errors": [
            {
              "extensions": {
                "affectedPath": "topProducts.inStock",
                "code": "UNAUTHORIZED_FIELD_OR_TYPE"
              },
              "message": "Unauthorized field or type"
            }
          ]
        }
        "#);
    }

    /// `x-user-role: admin` grants every policy the operation depends on -
    /// `inStock` comes back normally.
    #[ntex::test]
    async fn allows_the_gated_field_with_the_admin_role() {
        let subgraphs = TestSubgraphs::builder().build().start().await;

        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .file_config("../plugin_examples/custom_policy/router.config.yaml")
            .register_plugin::<crate::plugin::CustomPolicyPlugin>()
            .build()
            .start()
            .await;

        let response = router
            .send_graphql_request(
                "{ topProducts(first: 1) { upc name inStock } }",
                None,
                e2e::some_header_map! {
                    "x-user-role" => "admin"
                },
            )
            .await;

        e2e::insta::assert_snapshot!(response.json_body_string_pretty_stable().await, @r#"
        {
          "data": {
            "topProducts": [
              {
                "inStock": true,
                "name": "Table",
                "upc": "1"
              }
            ]
          }
        }
        "#);
    }
}
