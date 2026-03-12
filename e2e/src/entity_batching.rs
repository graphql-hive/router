#[cfg(test)]
mod entity_batching_e2e_tests {
    use std::collections::BTreeMap;
    use std::time::Duration;

    use sonic_rs::JsonValueTrait;

    use crate::testkit::{ClientResponseExt, TestRouter, TestSubgraphs};

    #[ntex::test]
    async fn measure_bench_operation_subgraph_calls() {
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

        let json_body = res.json_body().await;

        assert!(json_body["data"].is_object());
        assert!(
            json_body["errors"].is_null(),
            "expected no GraphQL errors in bench operation response"
        );

        let mut subgraph_request_counts: BTreeMap<&str, usize> = BTreeMap::new();
        for subgraph in ["accounts", "inventory", "products", "reviews"] {
            let request_count = subgraphs
                .get_requests_log(subgraph)
                .map_or(0, |requests| requests.len());
            subgraph_request_counts.insert(subgraph, request_count);
        }

        insta::assert_snapshot!(
            sonic_rs::to_string_pretty(&subgraph_request_counts).unwrap(),
            @r#"
        {
          "accounts": 2,
          "inventory": 2,
          "products": 2,
          "reviews": 1
        }
        "#
        );
    }
}
