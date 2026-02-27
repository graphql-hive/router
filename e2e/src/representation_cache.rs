#[cfg(test)]
mod representation_cache_e2e_tests {
    use std::collections::BTreeMap;

    use ntex::web::test;
    use sonic_rs::{from_slice, JsonValueTrait, Value};

    use crate::testkit::{
        init_graphql_request, init_router_from_config_inline, wait_for_readiness, SubgraphsServer,
    };

    #[ntex::test]
    async fn should_keep_stable_subgraph_call_counts_for_bench_operation() {
        let subgraphs_server = SubgraphsServer::start().await;

        let router = init_router_from_config_inline(
            r#"
            supergraph:
              source: file
              path: supergraph.graphql
            traffic_shaping:
              all:
                dedupe_enabled: false
            "#,
        )
        .await
        .expect("failed to start router");

        wait_for_readiness(&router.app).await;

        let req =
            init_graphql_request(include_str!("../../bench/operation.graphql"), None).to_request();
        let res = test::call_service(&router.app, req).await;

        assert!(res.status().is_success(), "Expected 200 OK");

        let body = test::read_body(res).await;
        let json_body: Value = from_slice(&body).unwrap();

        assert!(json_body["data"].is_object());
        assert!(
            json_body["errors"].is_null(),
            "expected no GraphQL errors in bench operation response"
        );

        let mut subgraph_request_counts: BTreeMap<&str, usize> = BTreeMap::new();
        for subgraph in ["accounts", "inventory", "products", "reviews"] {
            let request_count = subgraphs_server
                .get_subgraph_requests_log(subgraph)
                .await
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
              "reviews": 2
            }
            "#
        );
    }
}
