//! Entity calls under `@include`/`@skip`, checked against every combination of the variables.
//!
//! Subgraphs are strict mocks (`strict_mock_subgraphs`): an entity lookup that selects no field,
//! or a field its JSON doesn't have, fails the request. So a test passing without errors means
//! the router never asked for something it didn't need. Each test also checks the response
//! data, the requests per subgraph, that a representations list never holds an entity twice,
//! and how many times each entity was looked up.
#[cfg(test)]
mod entity_conditions_e2e_tests {
    use std::collections::BTreeMap;

    use serde_json::{json, Value};

    use crate::testkit::{
        mock_subgraphs::strict_mock_subgraphs, ClientResponseExt, RequestLike, Started, TestRouter,
        TestSubgraphs,
    };

    const SKIP_ENTITY_FETCH: &str =
        "../bin/router/fixture/tests/skip-entity-fetch.supergraph.graphql";
    const UNION_MEMBERS: &str =
        "../bin/router/fixture/tests/union-member-entity-fields.supergraph.graphql";
    const OVERRIDE_REQUIRES: &str =
        "../bin/router/fixture/tests/override_requires.supergraph.graphql";
    const ISSUE_190: &str = "../bin/router/fixture/issues/190.supergraph.graphql";

    fn skip_entity_fetch_mocks() -> Value {
        json!({
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
        })
    }

    struct Setup {
        subgraphs: TestSubgraphs<Started>,
        router: TestRouter<Started>,
        lookups: crate::testkit::mock_subgraphs::Lookups,
    }

    /// Router config pointing each of `subgraphs` at the mock subgraphs, for a supergraph that
    /// leaves their URLs empty.
    fn override_urls(url: &str, subgraphs: &[&str]) -> String {
        let urls: String = subgraphs
            .iter()
            .map(|name| format!("    {name}:\n      url: \"{url}/{name}\"\n"))
            .collect();
        format!("override_subgraph_urls:\n  subgraphs:\n{urls}")
    }

    async fn start(supergraph: &str, mocks: Value, config: &str) -> Setup {
        start_with(supergraph, mocks, |_| config.to_string()).await
    }

    /// Like `start`, with the router config made once the subgraphs have an address, for a
    /// supergraph whose subgraph URLs `override_subgraph_urls` has to set.
    async fn start_with(
        supergraph: &str,
        mocks: Value,
        config: impl FnOnce(&str) -> String,
    ) -> Setup {
        let (mocks, lookups) = strict_mock_subgraphs(mocks);
        let subgraphs = TestSubgraphs::builder()
            .with_on_request(mocks)
            .build()
            .start()
            .await;
        let config = config(&subgraphs.url());
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(format!(
                "supergraph:\n  source: file\n  path: {supergraph}\n{config}"
            ))
            .build()
            .start()
            .await;
        Setup {
            subgraphs,
            router,
            lookups,
        }
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

        /// The requests each subgraph got since the last call. Fails when a representations
        /// list holds an entity twice.
        fn take_requests(&self, seen: &mut BTreeMap<String, usize>) -> BTreeMap<String, usize> {
            let mut new = BTreeMap::new();
            for subgraph in ["a", "b", "card", "product"] {
                let requests: Vec<RequestLike> = self
                    .subgraphs
                    .get_requests_log(subgraph)
                    .unwrap_or_default();
                let before = seen
                    .insert(subgraph.to_string(), requests.len())
                    .unwrap_or(0);
                for request in &requests[before..] {
                    let body: Value =
                        serde_json::from_slice(request.body.as_deref().unwrap()).unwrap();
                    for (name, list) in body["variables"].as_object().into_iter().flatten() {
                        if name != "representations" && !name.starts_with("__batch_reps") {
                            continue;
                        }
                        let list = list.as_array().unwrap();
                        for (index, entity) in list.iter().enumerate() {
                            assert!(
                                !list[..index].contains(entity),
                                "{subgraph} got {entity} twice in ${name}"
                            );
                        }
                    }
                }
                if requests.len() > before {
                    new.insert(subgraph.to_string(), requests.len() - before);
                }
            }
            new
        }
    }

    /// Every combination of `n` booleans.
    fn assignments(n: usize) -> impl Iterator<Item = Vec<bool>> {
        (0..1u32 << n).map(move |bits| (0..n).map(|bit| bits & (1 << bit) != 0).collect())
    }

    fn counts<const N: usize>(pairs: [(&str, usize); N]) -> BTreeMap<String, usize> {
        pairs
            .into_iter()
            .filter(|(_, count)| *count > 0)
            .map(|(name, count)| (name.to_string(), count))
            .collect()
    }

    #[ntex::test]
    /// Four fields of one product under their own conditions, one of them nested in another.
    /// `b` gets one request with one lookup when any of its fields is on, and none otherwise.
    async fn fields_of_one_entity_under_four_conditions() {
        let setup = start(SKIP_ENTITY_FETCH, skip_entity_fetch_mocks(), "").await;
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

        let mut seen = BTreeMap::new();
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

            let b = usize::from(d || !s);
            assert_eq!(
                setup.take_requests(&mut seen),
                counts([("a", 1), ("b", b)]),
                "{case}"
            );
            assert_eq!(
                setup.lookups.take(),
                counts([("b/Product:p1", b)]),
                "{case}"
            );
        }
    }

    #[ntex::test]
    /// `@include` and `@skip` on one field, and another condition on its sibling.
    async fn two_conditions_on_one_field_and_one_on_its_sibling() {
        let setup = start(SKIP_ENTITY_FETCH, skip_entity_fetch_mocks(), "").await;
        let query = r#"
          query($a: Boolean!, $b: Boolean!, $c: Boolean!) {
            product {
              id
              details @include(if: $a) @skip(if: $b) { color }
              inventory @include(if: $c) { inStock }
            }
          }
        "#;

        let mut seen = BTreeMap::new();
        for values in assignments(3) {
            let [a, b, c] = values[..] else {
                unreachable!()
            };
            let case = format!("a={a} b={b} c={c}");
            let response = setup.send(query, json!({ "a": a, "b": b, "c": c })).await;

            let mut product = json!({ "id": "p1" });
            if a && !b {
                product["details"] = json!({ "color": "red" });
            }
            if c {
                product["inventory"] = json!({ "inStock": true });
            }
            assert_eq!(
                response,
                json!({ "data": { "product": product } }),
                "{case}"
            );

            let calls = usize::from((a && !b) || c);
            assert_eq!(
                setup.take_requests(&mut seen),
                counts([("a", 1), ("b", calls)]),
                "{case}"
            );
            assert_eq!(
                setup.lookups.take(),
                counts([("b/Product:p1", calls)]),
                "{case}"
            );
        }
    }

    #[ntex::test]
    /// `@include(if: $x) @skip(if: $x)` on both fields of `b`, so `b` is never asked for
    /// anything. The strict mock fails a lookup that selects nothing.
    async fn fields_that_are_never_asked_for_send_no_request() {
        let setup = start(SKIP_ENTITY_FETCH, skip_entity_fetch_mocks(), "").await;
        let query = r#"
          query($x: Boolean!, $y: Boolean!) {
            product {
              id
              details @include(if: $x) @skip(if: $x) { color }
              inventory @include(if: $y) @skip(if: $y) { inStock }
            }
          }
        "#;

        let mut seen = BTreeMap::new();
        for values in assignments(2) {
            let [x, y] = values[..] else { unreachable!() };
            let case = format!("x={x} y={y}");
            let response = setup.send(query, json!({ "x": x, "y": y })).await;

            assert_eq!(
                response,
                json!({ "data": { "product": { "id": "p1" } } }),
                "{case}"
            );
            assert_eq!(setup.take_requests(&mut seen), counts([("a", 1)]), "{case}");
            assert_eq!(setup.lookups.take(), counts([]), "{case}");
        }
    }

    #[ntex::test]
    /// A condition on the parent field and one on its child. With the parent off, not even the
    /// root fetch goes out, there's nothing left to ask for.
    async fn condition_on_the_parent_and_on_the_child() {
        let setup = start(SKIP_ENTITY_FETCH, skip_entity_fetch_mocks(), "").await;
        let query = r#"
          query($p: Boolean!, $d: Boolean!) {
            product @include(if: $p) {
              id
              details @include(if: $d) { color }
            }
          }
        "#;

        let mut seen = BTreeMap::new();
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
            assert_eq!(
                setup.take_requests(&mut seen),
                counts([("a", usize::from(p)), ("b", usize::from(p && d))]),
                "{case}"
            );
        }
    }

    const SAME_SHAPE_AT_TWO_PATHS: &str = r#"
      query($a: Boolean!, $b: Boolean!) {
        first: product { id details @include(if: $a) { color } }
        second: product { id details @include(if: $b) { color } }
      }
    "#;

    #[ntex::test]
    /// The same product at two paths, asked for the same field under other conditions. That's
    /// one alias with a group of paths per condition, so the product is looked up once.
    async fn same_entity_at_two_paths_is_looked_up_once() {
        let setup = start(SKIP_ENTITY_FETCH, skip_entity_fetch_mocks(), "").await;

        let mut seen = BTreeMap::new();
        for values in assignments(2) {
            let [a, b] = values[..] else { unreachable!() };
            let case = format!("a={a} b={b}");
            let response = setup
                .send(SAME_SHAPE_AT_TWO_PATHS, json!({ "a": a, "b": b }))
                .await;

            let product = |on: bool| {
                if on {
                    json!({ "id": "p1", "details": { "color": "red" } })
                } else {
                    json!({ "id": "p1" })
                }
            };
            assert_eq!(
                response,
                json!({ "data": { "first": product(a), "second": product(b) } }),
                "{case}"
            );

            let calls = usize::from(a || b);
            assert_eq!(
                setup.take_requests(&mut seen),
                counts([("a", 1), ("b", calls)]),
                "{case}"
            );
            assert_eq!(
                setup.lookups.take(),
                counts([("b/Product:p1", calls)]),
                "{case}"
            );
        }
    }

    #[ntex::test]
    /// An entity error lands at every active place the entity sits, and nowhere else.
    async fn entity_error_lands_only_at_active_paths() {
        let mut mocks = skip_entity_fetch_mocks();
        mocks["b"]["entities"][0]["__error"] = json!("boom");
        let setup = start(SKIP_ENTITY_FETCH, mocks, "").await;

        for values in assignments(2) {
            let [a, b] = values[..] else { unreachable!() };
            let case = format!("a={a} b={b}");
            let response = setup
                .send(SAME_SHAPE_AT_TWO_PATHS, json!({ "a": a, "b": b }))
                .await;

            let mut places: Vec<&str> = response["errors"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|error| error["path"][0].as_str().unwrap())
                .collect();
            places.sort();
            let expected: Vec<&str> = [("first", a), ("second", b)]
                .into_iter()
                .filter(|(_, on)| *on)
                .map(|(place, _)| place)
                .collect();
            assert_eq!(places, expected, "{case}: {response}");
        }
    }

    #[ntex::test]
    /// https://github.com/graphql-hive/router/issues/1732
    ///
    /// Each union member asks for its own field of `Product`, two of them under conditions.
    /// Each product only has its own field, so asking it for another one fails.
    async fn union_members_ask_only_for_their_own_fields() {
        let mocks = json!({
            "card": {
                "query": { "card": {
                    "__typename": "ProductCard",
                    "id": "c1",
                    "components": [
                        { "__typename": "ComponentA", "product": { "__typename": "Product", "id": "p1" } },
                        { "__typename": "ComponentB", "product": { "__typename": "Product", "id": "p2" } },
                        { "__typename": "ComponentC", "product": { "__typename": "Product", "id": "p3" } }
                    ]
                } }
            },
            "product": {
                "entities": [
                    { "__typename": "Product", "id": "p1", "a": "a1" },
                    { "__typename": "Product", "id": "p2", "b": "b2" },
                    { "__typename": "Product", "id": "p3", "c": "c3" }
                ]
            }
        });
        let setup = start(UNION_MEMBERS, mocks, "").await;
        let query = r#"
          query($x: Boolean!, $y: Boolean!) {
            card {
              components {
                ... on ComponentA { product { a @include(if: $x) } }
                ... on ComponentB { product { b @include(if: $y) } }
                ... on ComponentC { product { c } }
              }
            }
          }
        "#;

        let mut seen = BTreeMap::new();
        for values in assignments(2) {
            let [x, y] = values[..] else { unreachable!() };
            let case = format!("x={x} y={y}");
            let response = setup.send(query, json!({ "x": x, "y": y })).await;

            let a = if x { json!({ "a": "a1" }) } else { json!({}) };
            let b = if y { json!({ "b": "b2" }) } else { json!({}) };
            assert_eq!(
                response,
                json!({ "data": { "card": { "components": [
                    { "product": a },
                    { "product": b },
                    { "product": { "c": "c3" } }
                ] } } }),
                "{case}"
            );
            assert_eq!(
                setup.take_requests(&mut seen),
                counts([("card", 1), ("product", 1)]),
                "{case}"
            );
            assert_eq!(
                setup.lookups.take(),
                counts([
                    ("product/Product:p1", usize::from(x)),
                    ("product/Product:p2", usize::from(y)),
                    ("product/Product:p3", 1),
                ]),
                "{case}"
            );
        }
    }

    #[ntex::test]
    /// `userInB`'s calls to `a` and `c` sit under `$x`, next to an unconditional one for `userInC`.
    /// The plan has them as `Parallel { Include(if: $x) { Parallel { a, c } }, .. }`, and the
    /// executor dropped a `Parallel` nested like that, leaving `userInB` null without an error.
    async fn entity_calls_under_one_condition_next_to_others_run() {
        let mocks = json!({
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
        });
        let setup = start_with(OVERRIDE_REQUIRES, mocks, |url| {
            override_urls(url, &["a", "b", "c"])
        })
        .await;
        let query = r#"
          query($x: Boolean!) {
            userInC { cName }
            userInB @include(if: $x) { cName aName }
          }
        "#;

        for x in [true, false] {
            let response = setup.send(query, json!({ "x": x })).await;
            let mut data = json!({ "userInC": { "cName": "c-Bob" } });
            if x {
                data["userInB"] = json!({ "cName": "c-Ann", "aName": "a-Ann" });
            }
            assert_eq!(response, json!({ "data": data }), "x={x}");
        }
    }

    #[ntex::test]
    /// `results` once on its own and once under `$a` with `... on Product { id }`. The projection
    /// picks the `id` of a `Product` by `__typename`, which the subgraph has to send with `$a` off
    /// too, or `id` goes missing. The mock only sends what it's asked for.
    async fn abstract_field_selected_again_under_a_condition_keeps_its_fields() {
        let mocks = json!({
            "recommender": {
                "query": { "recommender": {
                    "__typename": "Recommender",
                    "results": [{ "__typename": "Product", "id": "p1" }]
                } }
            }
        });
        let setup = start_with(ISSUE_190, mocks, |url| override_urls(url, &["recommender"])).await;
        let query = r#"
          query($a: Boolean!) {
            recommender {
              results { id }
              results @include(if: $a) { ... on Product { id } }
            }
          }
        "#;

        for a in [false, true] {
            let response = setup.send(query, json!({ "a": a })).await;
            assert_eq!(
                response,
                json!({ "data": { "recommender": { "results": [{ "id": "p1" }] } } }),
                "a={a}"
            );
        }
    }

    #[ntex::test]
    /// A condition variable with a default the client leaves out gets the default.
    async fn omitted_condition_variable_gets_its_default() {
        let setup = start(SKIP_ENTITY_FETCH, skip_entity_fetch_mocks(), "").await;
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
        let mut seen = BTreeMap::new();
        assert_eq!(setup.take_requests(&mut seen), counts([("a", 1), ("b", 1)]));
    }

    #[ntex::test]
    /// The estimate counts the entity call when any of its fields is on, not only when all
    /// of them are, and leaves it out with all of them off.
    async fn estimate_counts_the_entity_call_when_any_field_is_on() {
        let setup = start(
            SKIP_ENTITY_FETCH,
            skip_entity_fetch_mocks(),
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
