#[cfg(test)]
mod entity_batching_e2e_tests {
    use std::collections::BTreeMap;

    use serde_json::json;
    use sonic_rs::JsonValueTrait;

    use crate::testkit::{
        mock_subgraphs::mock_subgraphs, ClientResponseExt, EnvVarsGuard, TestRouter, TestSubgraphs,
    };

    #[ntex::test]
    async fn measure_bench_operation_subgraph_calls() {
        // With entity reuse, most of these batches never go out, so it's off here.
        // See `entity_reuse.rs` for the counts with it on.
        let _env = EnvVarsGuard::new()
            .set("HIVE_ROUTER_EXPERIMENTAL_ENTITY_REUSE", "0")
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

    #[ntex::test]
    async fn max_aliases_per_request_splits_batches() {
        let mut data = Vec::new();
        let mut counts = Vec::new();
        for max_aliases in [15, 1] {
            let subgraphs = TestSubgraphs::builder().build().start().await;
            let router = TestRouter::builder()
                .with_subgraphs(&subgraphs)
                .inline_config(format!(
                    r#"
                      supergraph:
                        source: file
                        path: supergraph.graphql
                      traffic_shaping:
                        all:
                          dedupe_enabled: false
                          max_aliases_per_request: {max_aliases}
                      "#,
                ))
                .build()
                .start()
                .await;

            let res = router
                .send_graphql_request(include_str!("../../bench/operation.graphql"), None, None)
                .await;
            assert!(res.status().is_success(), "Expected 200 OK");
            let json_body = res.json_body().await;
            assert!(json_body["errors"].is_null(), "expected no GraphQL errors");
            data.push(sonic_rs::to_string(&json_body["data"]).unwrap());

            let mut subgraph_request_counts: BTreeMap<&str, usize> = BTreeMap::new();
            for subgraph in ["accounts", "inventory", "products", "reviews"] {
                let request_count = subgraphs
                    .get_requests_log(subgraph)
                    .map_or(0, |requests| requests.len());
                subgraph_request_counts.insert(subgraph, request_count);
            }
            counts.push(subgraph_request_counts);
        }

        assert_eq!(
            data[0], data[1],
            "splitting batches must not change the data"
        );
        insta::assert_snapshot!(sonic_rs::to_string_pretty(&counts).unwrap(), @r#"
        [
          {
            "accounts": 2,
            "inventory": 2,
            "products": 2,
            "reviews": 1
          },
          {
            "accounts": 2,
            "inventory": 2,
            "products": 2,
            "reviews": 2
          }
        ]
        "#);
    }

    #[ntex::test]
    /// `first` calls `c` under `$x`, `second` under `$x && !$y`: one request, an alias each, `[]`
    /// for the one that's off, and no request with both off.
    async fn entity_calls_under_other_conditions_share_one_request() {
        let subgraphs = TestSubgraphs::builder()
            .with_on_request(mock_subgraphs(json!({
                "b": {
                    "query": { "userInB": { "__typename": "User", "id": "u1", "name": "Ann" } }
                },
                "c": {
                    "entities": [
                        { "__typename": "User", "id": "u1", "name": "Ann", "cName": "c-Ann" }
                    ]
                }
            })))
            .build()
            .start()
            .await;
        // the fixture leaves subgraph URLs empty
        let urls: String = ["a", "b", "c"]
            .iter()
            .map(|name| format!("    {name}:\n      url: \"{}/{name}\"\n", subgraphs.url()))
            .collect();
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(format!(
                "supergraph:\n  source: file\n  path: ../bin/router/fixture/tests/override_requires.supergraph.graphql\noverride_subgraph_urls:\n  subgraphs:\n{urls}"
            ))
            .build()
            .start()
            .await;
        let query = r#"
          query($x: Boolean!, $y: Boolean!) {
            first: userInB { id name ... @include(if: $x) { cName } }
            second: userInB { id name ... @include(if: $x) { ... @skip(if: $y) { cName } } }
          }
        "#;
        let send = |x: bool, y: bool| {
            let variables = sonic_rs::from_str(&json!({ "x": x, "y": y }).to_string()).unwrap();
            router.send_graphql_request(query, Some(variables), None)
        };
        let user = json!({ "id": "u1", "name": "Ann" });

        let body: serde_json::Value =
            serde_json::from_str(&send(true, true).await.string_body().await).unwrap();
        let first = json!({ "id": "u1", "name": "Ann", "cName": "c-Ann" });
        assert_eq!(body, json!({ "data": { "first": first, "second": user } }));
        let requests = subgraphs.get_requests_log("c").unwrap_or_default();
        assert_eq!(requests.len(), 1);
        let request: serde_json::Value =
            serde_json::from_slice(requests[0].body.as_ref().unwrap()).unwrap();
        let mut lists: Vec<usize> = request["variables"]
            .as_object()
            .unwrap()
            .iter()
            .filter(|(name, _)| name.starts_with("__batch_reps_"))
            .map(|(_, list)| list.as_array().unwrap().len())
            .collect();
        lists.sort();
        assert_eq!(lists, [0, 1]);

        let body: serde_json::Value =
            serde_json::from_str(&send(false, false).await.string_body().await).unwrap();
        assert_eq!(body, json!({ "data": { "first": user, "second": user } }));
        assert_eq!(subgraphs.get_requests_log("c").unwrap_or_default().len(), 1);
    }
}
