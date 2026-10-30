#[cfg(test)]
mod entity_batching_e2e_tests {
    use std::collections::BTreeMap;

    use serde_json::json;
    use sonic_rs::JsonValueTrait;

    use crate::testkit::{
        mock_subgraphs::mock_subgraphs, ClientResponseExt, TestRouter, TestSubgraphs,
    };

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
    /// `userInB`'s calls to `a` and `c` sit under `$x`, next to an unconditional one for `userInC`.
    /// The plan has them as `Parallel { Include(if: $x) { Parallel { a, c } }, .. }`, and the
    /// executor used to drop a `Parallel` nested like that, leaving `userInB` null without an error.
    async fn entity_calls_under_one_condition_next_to_others_run() {
        let subgraphs = TestSubgraphs::builder()
            .with_on_request(mock_subgraphs(json!({
                "b": {
                    "query": { "userInB": { "__typename": "User", "id": "u1", "name": "Ann" } },
                    "entities": [{ "__typename": "User", "id": "u2", "name": "Bob" }]
                },
                "c": {
                    "query": { "userInC": { "__typename": "User", "id": "u2" } },
                    "entities": [
                        { "__typename": "User", "id": "u1", "name": "Ann", "cName": "c-Ann" },
                        { "__typename": "User", "id": "u2", "name": "Bob", "cName": "c-Bob" }
                    ]
                },
                "a": {
                    "entities": [{ "__typename": "User", "id": "u1", "name": "Ann", "aName": "a-Ann" }]
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
          query($x: Boolean!) {
            userInC { cName }
            userInB @include(if: $x) { cName aName }
          }
        "#;

        for x in [true, false] {
            let variables = sonic_rs::from_str(&json!({ "x": x }).to_string()).unwrap();
            let res = router
                .send_graphql_request(query, Some(variables), None)
                .await;
            let body: serde_json::Value = serde_json::from_str(&res.string_body().await).unwrap();
            let mut data = json!({ "userInC": { "cName": "c-Bob" } });
            if x {
                data["userInB"] = json!({ "cName": "c-Ann", "aName": "a-Ann" });
            }
            assert_eq!(body, json!({ "data": data }), "x={x}");
        }
    }
}
