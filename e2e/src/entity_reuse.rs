#[cfg(test)]
mod entity_reuse_e2e_tests {
    use std::collections::BTreeMap;

    use crate::testkit::{ClientResponseExt, EnvVarsGuard, TestRouter, TestSubgraphs};

    // In the bench operation, every product, user and inventory entity the last two waves ask
    // for was already fetched by `topProducts`, `users` and the first inventory fetch.
    // With reuse on, those fetches copy them instead of calling the subgraphs again.
    #[ntex::test]
    async fn bench_operation_copies_entities_it_already_has() {
        let _env = EnvVarsGuard::new()
            .set("HIVE_ROUTER_EXPERIMENTAL_ENTITY_REUSE", "1")
            .apply()
            .await;
        let subgraphs = TestSubgraphs::builder().build().start().await;

        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                  supergraph:
                    source: file
                    path: supergraph.graphql
                  traffic_shaping:
                    all:
                      dedupe_enabled: false
                  "#,
            )
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(include_str!("../../bench/operation.graphql"), None, None)
            .await;
        assert!(res.status().is_success(), "Expected 200 OK");

        let body: sonic_rs::Value = res.json_body().await;
        let expected: sonic_rs::Value =
            sonic_rs::from_str(include_str!("../../bench/expected_response.json")).unwrap();
        assert_eq!(
            body, expected,
            "copied entities have to give the same response as fetched ones"
        );

        let mut subgraph_request_counts: BTreeMap<&str, usize> = BTreeMap::new();
        for subgraph in ["accounts", "inventory", "products", "reviews"] {
            let request_count = subgraphs
                .get_requests_log(subgraph)
                .map_or(0, |requests| requests.len());
            subgraph_request_counts.insert(subgraph, request_count);
        }

        // Without reuse: accounts 2, inventory 2, products 2, reviews 1.
        insta::assert_snapshot!(
            sonic_rs::to_string_pretty(&subgraph_request_counts).unwrap(),
            @r#"
        {
          "accounts": 1,
          "inventory": 1,
          "products": 1,
          "reviews": 1
        }
        "#
        );
    }
}
