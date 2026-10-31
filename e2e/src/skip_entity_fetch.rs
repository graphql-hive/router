//! A fetch whose fields are all switched off by `@include`/`@skip` isn't sent, see
//! https://github.com/graphql-hive/router/issues/1733. Each test runs every combination of its
//! variables and checks the response and how many requests each subgraph got.
#[cfg(test)]
mod skip_entity_fetch_e2e_tests {
    use serde_json::{json, Value};

    use crate::testkit::{
        mock_subgraphs::mock_subgraphs, ClientResponseExt, Started, TestRouter, TestSubgraphs,
    };

    struct Setup {
        subgraphs: TestSubgraphs<Started>,
        router: TestRouter<Started>,
    }

    async fn start(config: &str) -> Setup {
        let subgraphs = TestSubgraphs::builder()
            .with_on_request(mock_subgraphs(json!({
                "a": {
                    "query": { "product": { "__typename": "Product", "id": "p1", "name": "Phone" } }
                },
                "b": {
                    "entities": [{
                        "__typename": "Product",
                        "id": "p1",
                        "details": { "color": "red", "size": "L" },
                        "inventory": { "inStock": true, "warehouse": "W1" }
                    }]
                }
            })))
            .build()
            .start()
            .await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(format!(
                "supergraph:\n  source: file\n  path: ../bin/router/fixture/tests/skip-entity-fetch.supergraph.graphql\n{config}"
            ))
            .build()
            .start()
            .await;
        Setup { subgraphs, router }
    }

    impl Setup {
        async fn send(&self, query: &str, variables: Value) -> Value {
            let variables = sonic_rs::from_str(&variables.to_string()).unwrap();
            let response = self
                .router
                .send_graphql_request(query, Some(variables), None)
                .await;
            serde_json::from_str(&response.string_body().await).unwrap()
        }

        /// Requests to `a` and `b` so far.
        fn requests(&self) -> [usize; 2] {
            ["a", "b"].map(|name| {
                self.subgraphs
                    .get_requests_log(name)
                    .map_or(0, |requests| requests.len())
            })
        }
    }

    /// Every combination of `n` booleans.
    fn assignments(n: usize) -> impl Iterator<Item = Vec<bool>> {
        (0..1u32 << n).map(move |bits| (0..n).map(|bit| bits & (1 << bit) != 0).collect())
    }

    #[ntex::test]
    /// Four fields of one product under their own conditions, one of them nested in another.
    /// `b` gets a request when any of its fields is on, and none otherwise.
    async fn fields_of_one_entity_under_four_conditions() {
        let setup = start("").await;
        let query = r#"
          query($n: Boolean!, $d: Boolean!, $s: Boolean!, $w: Boolean!) {
            product {
              id
              name @include(if: $n)
              details @include(if: $d) { color size }
              inventory @skip(if: $s) { inStock warehouse @include(if: $w) }
            }
          }
        "#;

        let mut before = setup.requests();
        for values in assignments(4) {
            let [n, d, s, w] = values[..] else {
                unreachable!()
            };
            let case = format!("n={n} d={d} s={s} w={w}");
            let response = setup
                .send(query, json!({ "n": n, "d": d, "s": s, "w": w }))
                .await;

            let mut product = json!({ "id": "p1" });
            if n {
                product["name"] = json!("Phone");
            }
            if d {
                product["details"] = json!({ "color": "red", "size": "L" });
            }
            if !s {
                product["inventory"] = json!({ "inStock": true });
                if w {
                    product["inventory"]["warehouse"] = json!("W1");
                }
            }
            assert_eq!(
                response,
                json!({ "data": { "product": product } }),
                "{case}"
            );

            let after = setup.requests();
            let b = usize::from(d || !s);
            assert_eq!(
                [after[0] - before[0], after[1] - before[1]],
                [1, b],
                "{case}"
            );
            before = after;
        }
    }

    #[ntex::test]
    /// `@include(if: $x) @skip(if: $x)` on both fields of `b`, so `b` is never asked for
    /// anything.
    async fn fields_that_are_never_asked_for_send_no_request() {
        let setup = start("").await;
        let query = r#"
          query($x: Boolean!, $y: Boolean!) {
            product {
              id
              details @include(if: $x) @skip(if: $x) { color }
              inventory @include(if: $y) @skip(if: $y) { inStock }
            }
          }
        "#;

        for values in assignments(2) {
            let [x, y] = values[..] else { unreachable!() };
            let response = setup.send(query, json!({ "x": x, "y": y })).await;
            assert_eq!(
                response,
                json!({ "data": { "product": { "id": "p1" } } }),
                "x={x} y={y}"
            );
        }
        assert_eq!(setup.requests(), [4, 0]);
    }

    #[ntex::test]
    /// A condition on the parent field and one on its child. With the parent off, not even the
    /// root fetch goes out, there's nothing left to ask for.
    async fn condition_on_the_parent_and_on_the_child() {
        let setup = start("").await;
        let query = r#"
          query($p: Boolean!, $d: Boolean!) {
            product @include(if: $p) {
              id
              details @include(if: $d) { color }
            }
          }
        "#;

        let mut before = setup.requests();
        for values in assignments(2) {
            let [p, d] = values[..] else { unreachable!() };
            let case = format!("p={p} d={d}");
            let response = setup.send(query, json!({ "p": p, "d": d })).await;

            let data = match (p, d) {
                (false, _) => json!({}),
                (true, false) => json!({ "product": { "id": "p1" } }),
                (true, true) => json!({ "product": { "id": "p1", "details": { "color": "red" } } }),
            };
            assert_eq!(response, json!({ "data": data }), "{case}");

            let after = setup.requests();
            assert_eq!(
                [after[0] - before[0], after[1] - before[1]],
                [usize::from(p), usize::from(p && d)],
                "{case}"
            );
            before = after;
        }
    }

    #[ntex::test]
    /// A condition variable with a default the client leaves out gets the default.
    async fn omitted_condition_variable_gets_its_default() {
        let setup = start("").await;
        let query = r#"
          query($d: Boolean = true, $i: Boolean = false) {
            product {
              id
              details @include(if: $d) { color }
              inventory @include(if: $i) { inStock }
            }
          }
        "#;

        let response = setup.send(query, json!({})).await;
        assert_eq!(
            response,
            json!({ "data": { "product": { "id": "p1", "details": { "color": "red" } } } })
        );
        assert_eq!(setup.requests(), [1, 1]);
    }

    #[ntex::test]
    /// The estimate counts the entity call when any of its fields is on, not only when all
    /// of them are, and leaves it out with all of them off.
    async fn estimate_counts_the_entity_call_when_any_field_is_on() {
        let setup = start(
            r#"
demand_control:
  enabled: true
  operation_cost:
    max: 1000
    mode: measure
    expose_headers:
      estimated: true
  default_list_size:
    all: 1
  subgraphs_budget:
    mode: measure
"#,
        )
        .await;
        let query = r#"
          query($skipDetails: Boolean!, $withInventory: Boolean!) {
            product {
              id
              details @skip(if: $skipDetails) { color }
              inventory @include(if: $withInventory) { inStock }
            }
          }
        "#;
        let estimate = |query: &'static str, variables: Value| {
            let router = &setup.router;
            async move {
                let variables = sonic_rs::from_str(&variables.to_string()).unwrap();
                router
                    .send_graphql_request(query, Some(variables), None)
                    .await
                    .cost_header("x-cost-estimated")
                    .expect("estimated cost header should be present")
            }
        };

        let baseline = estimate("{ product { id } }", json!({})).await;
        let off = estimate(
            query,
            json!({ "skipDetails": true, "withInventory": false }),
        )
        .await;
        let details = estimate(
            query,
            json!({ "skipDetails": false, "withInventory": false }),
        )
        .await;
        let inventory =
            estimate(query, json!({ "skipDetails": true, "withInventory": true })).await;
        let both = estimate(
            query,
            json!({ "skipDetails": false, "withInventory": true }),
        )
        .await;

        assert_eq!(off, baseline, "nothing from b, nothing counted");
        assert!(details > baseline, "{details} vs {baseline}");
        assert!(inventory > baseline, "{inventory} vs {baseline}");
        assert!(both >= details.max(inventory), "{both}");
    }
}
