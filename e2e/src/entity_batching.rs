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
    #[ntex::test]
    /// `first` and `second` both call `c` for `cName`, one under `$x`, one under `!$y`. Both users
    /// and their keys are always there, so only the alias's condition keeps an off one from
    /// collecting. They go out as one request with an alias each, `@include`/`@skip` on it, and an
    /// alias that's off sends no representations. With both off there's no request.
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
            second: userInB { id name ... @skip(if: $y) { cName } }
          }
        "#;
        let requests_to_c = || subgraphs.get_requests_log("c").unwrap_or_default();

        for (x, y) in [(true, true), (true, false), (false, true), (false, false)] {
            let before = requests_to_c().len();
            let variables = sonic_rs::from_str(&json!({ "x": x, "y": y }).to_string()).unwrap();
            let res = router
                .send_graphql_request(query, Some(variables), None)
                .await;
            let body: serde_json::Value = serde_json::from_str(&res.string_body().await).unwrap();
            let user = json!({ "id": "u1", "name": "Ann" });
            let mut data = json!({ "first": user, "second": user });
            if x {
                data["first"]["cName"] = json!("c-Ann");
            }
            if !y {
                data["second"]["cName"] = json!("c-Ann");
            }
            assert_eq!(body, json!({ "data": data }), "x={x} y={y}");

            let requests = requests_to_c();
            let sent = &requests[before..];
            if !x && y {
                assert!(sent.is_empty(), "x={x} y={y}: no call is on");
                continue;
            }
            assert_eq!(sent.len(), 1, "x={x} y={y}: one request for both calls");
            let request: serde_json::Value =
                serde_json::from_slice(sent[0].body.as_ref().unwrap()).unwrap();
            let lists: Vec<usize> = request["variables"]
                .as_object()
                .unwrap()
                .iter()
                .filter(|(name, _)| name.starts_with("__batch_reps_"))
                .map(|(_, list)| list.as_array().unwrap().len())
                .collect();
            let mut expected = vec![usize::from(x), usize::from(!y)];
            let mut lists = lists;
            expected.sort();
            lists.sort();
            assert_eq!(lists, expected, "x={x} y={y}: representations per alias");
        }
    }
}
