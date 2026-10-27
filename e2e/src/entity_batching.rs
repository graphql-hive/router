#[cfg(test)]
mod entity_batching_e2e_tests {
    use std::collections::BTreeMap;

    use sonic_rs::{pointer, JsonValueTrait};

    use crate::testkit::{ClientResponseExt, TestRouter, TestSubgraphs};

    const CONDITIONED_BATCH_QUERY: &str = r#"
      query($reviews: Boolean!, $skipAuthor: Boolean!, $nestedReviews: Boolean!, $product: Boolean!) {
        topProducts {
          name
          reviews @include(if: $reviews) {
            author @skip(if: $skipAuthor) {
              name
              reviews @include(if: $nestedReviews) {
                body
                product @include(if: $product) {
                  name
                  shippingEstimate
                }
              }
            }
          }
        }
      }
    "#;

    const OPPOSITE_CONDITIONS_QUERY: &str = r#"
      query($condition: Boolean!) {
        topProducts @include(if: $condition) {
          name
        }
        users @skip(if: $condition) {
          name
        }
      }
    "#;

    const ALIASED_CONDITIONED_BATCH_QUERY: &str = r#"
      query($showShipping: Boolean!) {
        first: topProducts(first: 1) {
          upc
          shippingEstimate @include(if: $showShipping)
        }
        second: topProducts(first: 2) {
          upc
          shippingEstimate @include(if: $showShipping)
        }
      }
    "#;

    const MIXED_CONDITION_BATCH_QUERY: &str = r#"
      query($showConditional: Boolean!) {
        first: topProducts(first: 1) {
          upc
          shippingEstimate @include(if: $showConditional)
        }
        second: topProducts(first: 2) {
          upc
          shippingEstimate @include(if: $showConditional)
        }
        always: topProducts(first: 3) {
          upc
          shippingEstimate
        }
      }
    "#;

    #[ntex::test]
    /// `always` needs `shippingEstimate` every time, `first` and `second` only under
    /// `$showConditional`. They still go to inventory in one request, the conditional ones get
    /// their own alias:
    ///
    /// ```text
    /// Sequence
    /// ├─ Fetch(products)                         first, second, always
    /// └─ BatchFetch(inventory)
    ///    ├─ _e0                                  always.@
    ///    └─ _e1 @include(if: $showConditional)   first.@, second.@
    /// ```
    ///
    /// ```text
    /// $showConditional  inventory requests  $__batch_reps_0  $__batch_reps_1
    /// true              1                   always           first + second
    /// false             1                   always           [] (nothing collected)
    /// ```
    async fn conditional_fetches_batch_with_unconditional_entity_fetch() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                  supergraph:
                    source: file
                    path: supergraph.graphql
                  "#,
            )
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(
                MIXED_CONDITION_BATCH_QUERY,
                Some(sonic_rs::json!({ "showConditional": true })),
                None,
            )
            .await;
        assert!(res.status().is_success(), "Expected 200 OK");
        let json_body = res.json_body().await;
        assert!(
            json_body["errors"].is_null(),
            "expected no GraphQL errors: {json_body}"
        );
        for alias in ["first", "second", "always"] {
            assert!(json_body["data"][alias].is_array());
            assert!(
                json_body
                    .pointer(&pointer!["data", alias, 0, "shippingEstimate"])
                    .is_some(),
                "expected shippingEstimate for {alias}: {json_body}"
            );
        }

        // One request, the conditional alias joins the unconditional one.
        let requests = subgraphs
            .get_requests_log("inventory")
            .expect("expected inventory subgraph requests");
        assert_eq!(requests.len(), 1, "expected one batched inventory request");

        let res = router
            .send_graphql_request(
                MIXED_CONDITION_BATCH_QUERY,
                Some(sonic_rs::json!({ "showConditional": false })),
                None,
            )
            .await;
        assert!(res.status().is_success(), "Expected 200 OK");
        let json_body = res.json_body().await;
        assert!(
            json_body["errors"].is_null(),
            "expected no GraphQL errors: {json_body}"
        );
        assert!(
            json_body
                .pointer(&pointer!["data", "always", 0, "shippingEstimate"])
                .is_some(),
            "the unconditional entity result should still be present: {json_body}"
        );
        assert!(
            json_body
                .pointer(&pointer!["data", "first", 0, "shippingEstimate"])
                .is_none(),
            "the first conditional entity result should be absent: {json_body}"
        );
        assert!(
            json_body
                .pointer(&pointer!["data", "second", 0, "shippingEstimate"])
                .is_none(),
            "the second conditional entity result should be absent: {json_body}"
        );
        // Still one request, and the conditional alias got no representations.
        let requests = subgraphs
            .get_requests_log("inventory")
            .expect("expected inventory subgraph requests");
        assert_eq!(requests.len(), 2, "expected one more inventory request");
        let body = requests[1]
            .body
            .as_ref()
            .expect("expected an inventory request body");
        let payload: sonic_rs::Value =
            sonic_rs::from_slice(body).expect("expected a JSON inventory request");
        assert_eq!(
            payload["variables"]["__batch_reps_1"],
            sonic_rs::json!([]),
            "the skipped alias should get an empty list: {payload}"
        );
        assert_ne!(
            payload["variables"]["__batch_reps_0"],
            sonic_rs::json!([]),
            "the unconditional alias should get representations: {payload}"
        );
    }

    #[ntex::test]
    /// `@include` and `@skip` on one field nest two conditions. The fetch for `first` still
    /// joins the one for `always`, and its alias gets both conditions:
    ///
    /// ```text
    /// Sequence
    /// ├─ Fetch(products)                           first, always
    /// └─ BatchFetch(inventory)
    ///    ├─ _e0                                    always.@
    ///    └─ _e1 @include(if: $a) @skip(if: $b)    first.@
    /// ```
    ///
    /// ```text
    /// $a     $b     inventory requests  first.shippingEstimate
    /// true   false  1                   yes
    /// true   true   1                   no
    /// false  false  1                   no
    /// false  true   1                   no
    /// ```
    async fn alias_under_nested_conditions_runs_only_when_all_allow_it() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                  supergraph:
                    source: file
                    path: supergraph.graphql
                  "#,
            )
            .build()
            .start()
            .await;

        let query = r#"
          query ($a: Boolean!, $b: Boolean!) {
            first: topProducts(first: 1) {
              upc
              shippingEstimate @include(if: $a) @skip(if: $b)
            }
            always: topProducts(first: 3) {
              upc
              shippingEstimate
            }
          }
        "#;

        let cases = [(true, false), (true, true), (false, false), (false, true)];
        for (sent, (a, b)) in cases.into_iter().enumerate() {
            let res = router
                .send_graphql_request(query, Some(sonic_rs::json!({ "a": a, "b": b })), None)
                .await;
            assert!(res.status().is_success(), "Expected 200 OK");
            let json_body = res.json_body().await;
            assert!(
                json_body["errors"].is_null(),
                "expected no GraphQL errors for a={a} b={b}: {json_body}"
            );
            assert!(
                json_body
                    .pointer(&pointer!["data", "always", 0, "shippingEstimate"])
                    .is_some(),
                "expected shippingEstimate for always, a={a} b={b}: {json_body}"
            );
            assert_eq!(
                json_body
                    .pointer(&pointer!["data", "first", 0, "shippingEstimate"])
                    .is_some(),
                a && !b,
                "shippingEstimate for first, a={a} b={b}: {json_body}"
            );

            let requests = subgraphs
                .get_requests_log("inventory")
                .expect("expected inventory subgraph requests");
            assert_eq!(
                requests.len(),
                sent + 1,
                "expected one inventory request for a={a} b={b}"
            );
        }
    }

    #[ntex::test]
    /// Both aliases need `shippingEstimate` only under `$showShipping`, so the whole batch sits
    /// under that one condition:
    ///
    /// ```text
    /// Sequence
    /// ├─ Fetch(products)              first, second
    /// └─ Include(if: $showShipping)
    ///    └─ BatchFetch(inventory)
    ///       └─ _e0                    first.@, second.@
    /// ```
    ///
    /// ```text
    /// $showShipping  inventory requests
    /// true           1, `_e0` fills both paths
    /// false          0, the batch doesn't run
    /// ```
    async fn executes_batch_fetch_under_a_condition() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                  supergraph:
                    source: file
                    path: supergraph.graphql
                  "#,
            )
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(
                ALIASED_CONDITIONED_BATCH_QUERY,
                Some(sonic_rs::json!({ "showShipping": true })),
                None,
            )
            .await;
        assert!(res.status().is_success(), "Expected 200 OK");
        let json_body = res.json_body().await;
        assert!(
            json_body["errors"].is_null(),
            "expected no GraphQL errors: {json_body}"
        );
        assert!(json_body["data"]["first"].is_array());
        assert!(json_body["data"]["second"].is_array());
        assert!(
            json_body
                .pointer(&pointer!["data", "first", 0, "shippingEstimate"])
                .is_some(),
            "expected first alias result from the batch: {json_body}"
        );
        assert!(
            json_body
                .pointer(&pointer!["data", "second", 0, "shippingEstimate"])
                .is_some(),
            "expected second alias result from the batch: {json_body}"
        );

        let requests = subgraphs
            .get_requests_log("inventory")
            .expect("expected an inventory subgraph request");
        assert_eq!(
            requests.len(),
            1,
            "same-condition entity fetches should be sent as one batched request"
        );
        let body = requests[0]
            .body
            .as_ref()
            .expect("expected the inventory request body");
        let payload: sonic_rs::Value =
            sonic_rs::from_slice(body).expect("expected a JSON inventory request");
        let operation = payload["query"]
            .as_str()
            .expect("expected the inventory request to contain an operation");
        assert!(
            operation.contains("_e0: _entities") && operation.contains("$__batch_reps_0"),
            "expected the inventory operation to be a BatchFetch: {operation}"
        );

        let res = router
            .send_graphql_request(
                ALIASED_CONDITIONED_BATCH_QUERY,
                Some(sonic_rs::json!({ "showShipping": false })),
                None,
            )
            .await;
        assert!(res.status().is_success(), "Expected 200 OK");
        let json_body = res.json_body().await;
        assert!(
            json_body["errors"].is_null(),
            "expected no GraphQL errors: {json_body}"
        );
        assert_eq!(
            subgraphs
                .get_requests_log("inventory")
                .expect("expected the first inventory request")
                .len(),
            1,
            "the BatchFetch must not execute when its condition is false"
        );
    }

    #[ntex::test]
    /// `topProducts` and `users` come from different subgraphs. The directives stay inside the
    /// root queries, so the plan has no condition nodes at all:
    ///
    /// ```text
    /// Parallel
    /// ├─ Fetch(products)   topProducts @include(if: $condition)
    /// └─ Fetch(accounts)   users @skip(if: $condition)
    /// ```
    ///
    /// ```text
    /// $condition  topProducts  users
    /// true        yes          no
    /// false       no           yes
    /// ```
    async fn root_include_and_skip_of_one_variable_return_one_field_each() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                  supergraph:
                    source: file
                    path: supergraph.graphql
                  "#,
            )
            .build()
            .start()
            .await;

        for (condition, expect_products, expect_accounts) in
            [(true, true, false), (false, false, true)]
        {
            let res = router
                .send_graphql_request(
                    OPPOSITE_CONDITIONS_QUERY,
                    Some(sonic_rs::json!({ "condition": condition })),
                    None,
                )
                .await;
            assert!(res.status().is_success(), "Expected 200 OK");
            let json_body = res.json_body().await;
            assert!(
                json_body["errors"].is_null(),
                "expected no GraphQL errors: {json_body}"
            );

            let data = &json_body["data"];
            assert_eq!(data["topProducts"].is_array(), expect_products);
            assert_eq!(data["users"].is_array(), expect_accounts);
        }
    }

    #[ntex::test]
    /// The inventory fetch sits under four conditions. It's the only inventory fetch in the plan,
    /// so it stays a plain `Flatten`, not a `BatchFetch`:
    ///
    /// ```text
    /// Sequence
    /// ├─ Fetch(products)                  topProducts
    /// ├─ Include(if: $reviews)
    /// │  └─ Flatten(reviews)              topProducts.@
    /// └─ Skip(if: $skipAuthor)
    ///    └─ Sequence
    ///       ├─ Parallel
    ///       │  ├─ Include(if: $nestedReviews)
    ///       │  │  └─ Include(if: $product)
    ///       │  │     └─ Flatten(products)   …author.reviews.@.product
    ///       │  └─ Flatten(accounts)         …reviews.@.author
    ///       └─ Include(if: $nestedReviews)
    ///          └─ Include(if: $product)
    ///             └─ Flatten(inventory)     …author.reviews.@.product
    /// ```
    ///
    /// All on: 1 inventory request, and `shippingEstimate` lands deep in the response.
    /// Any one of them off: no more inventory requests. With `$reviews: false` the `Skip` part
    /// still runs, but there are no reviews to walk, so nothing gets sent.
    async fn entity_fetch_under_nested_conditions_runs_only_when_all_allow_it() {
        let subgraphs = TestSubgraphs::builder().build().start().await;

        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                  supergraph:
                    source: file
                    path: supergraph.graphql
                  "#,
            )
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(
                CONDITIONED_BATCH_QUERY,
                Some(sonic_rs::json!({
                    "reviews": true,
                    "skipAuthor": false,
                    "nestedReviews": true,
                    "product": true,
                })),
                None,
            )
            .await;

        assert!(res.status().is_success(), "Expected 200 OK");
        let json_body = res.json_body().await;
        assert!(json_body["data"].is_object());
        assert!(
            json_body["errors"].is_null(),
            "expected no GraphQL errors in response: {json_body}"
        );
        assert!(
            json_body
                .pointer(&pointer![
                    "data",
                    "topProducts",
                    0,
                    "reviews",
                    0,
                    "author",
                    "reviews",
                    0,
                    "product",
                    "shippingEstimate"
                ])
                .is_some(),
            "expected the batched entity result to be merged at its nested response path: {json_body}"
        );

        let inventory_requests = subgraphs
            .get_requests_log("inventory")
            .expect("expected an inventory subgraph request");
        assert_eq!(
            inventory_requests.len(),
            1,
            "entity fetches under the same conditions should execute in one batch"
        );

        // Turning off the nested product branch must suppress its batched fetch as well.
        let res = router
            .send_graphql_request(
                CONDITIONED_BATCH_QUERY,
                Some(sonic_rs::json!({
                    "reviews": true,
                    "skipAuthor": false,
                    "nestedReviews": true,
                    "product": false,
                })),
                None,
            )
            .await;

        assert!(res.status().is_success(), "Expected 200 OK");
        let json_body = res.json_body().await;
        assert!(json_body["errors"].is_null());
        assert_eq!(
            subgraphs
                .get_requests_log("inventory")
                .expect("expected the first inventory request")
                .len(),
            1,
            "the conditional batch must not run when its nested @include is false"
        );

        // Also exercise the enclosing directives: none may leak the nested entity fetch.
        for variables in [
            sonic_rs::json!({
                "reviews": false,
                "skipAuthor": false,
                "nestedReviews": true,
                "product": true,
            }),
            sonic_rs::json!({
                "reviews": true,
                "skipAuthor": true,
                "nestedReviews": true,
                "product": true,
            }),
            sonic_rs::json!({
                "reviews": true,
                "skipAuthor": false,
                "nestedReviews": false,
                "product": true,
            }),
        ] {
            let res = router
                .send_graphql_request(CONDITIONED_BATCH_QUERY, Some(variables), None)
                .await;

            assert!(res.status().is_success(), "Expected 200 OK");
            let json_body = res.json_body().await;
            assert!(
                json_body["errors"].is_null(),
                "expected no GraphQL errors: {json_body}"
            );
            assert_eq!(
                subgraphs
                    .get_requests_log("inventory")
                    .expect("expected the first inventory request")
                    .len(),
                1,
                "a disabled enclosing condition must suppress the entity batch"
            );
        }
    }

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

    /// Products `1`, `2` and `3`, in `products` and in `inventory`.
    /// `topProducts(first: 2)` and `topProducts(first: 3)` overlap in `1` and `2`.
    fn mocked_products() -> serde_json::Value {
        let product = |upc: &str, price: i64, weight: i64| serde_json::json!({ "__typename": "Product", "upc": upc, "price": price, "weight": weight });
        let stock = |upc: &str, price: i64, weight: i64, in_stock: bool| {
            let mut entity = product(upc, price, weight);
            entity["inStock"] = in_stock.into();
            entity["shippingEstimate"] = (price * weight).into();
            entity
        };
        serde_json::json!({
            "products": {
                "query": {
                    "topProducts(first: 2)": [product("1", 10, 1), product("2", 20, 2)],
                    "topProducts(first: 3)": [product("1", 10, 1), product("2", 20, 2), product("3", 30, 3)]
                }
            },
            "inventory": {
                "entities": [stock("1", 10, 1, true), stock("2", 20, 2, false), stock("3", 30, 3, true)]
            }
        })
    }

    /// What one `_entities` alias of a batch asked for.
    #[derive(Debug, PartialEq)]
    struct SentAlias {
        fields: Vec<String>,
        /// The key of each representation, in the order they were sent.
        keys: Vec<String>,
    }

    /// The `_entities` aliases of a subgraph request, by alias.
    /// `key` is the key field of the entities.
    fn sent_aliases(body: &[u8], key: &str) -> BTreeMap<String, SentAlias> {
        use graphql_tools::parser::query::{Definition, OperationDefinition, Selection, Value};

        fn field_names(selections: &[Selection<'_, String>], out: &mut Vec<String>) {
            for selection in selections {
                match selection {
                    Selection::Field(field) => out.push(field.name.clone()),
                    Selection::InlineFragment(fragment) => {
                        field_names(&fragment.selection_set.items, out)
                    }
                    Selection::FragmentSpread(_) => panic!("expected inlined fragments"),
                }
            }
        }

        let body: serde_json::Value = serde_json::from_slice(body).unwrap();
        let document =
            graphql_tools::parser::parse_query::<String>(body["query"].as_str().unwrap()).unwrap();
        let selections = document
            .definitions
            .iter()
            .find_map(|definition| match definition {
                Definition::Operation(OperationDefinition::Query(query)) => {
                    Some(&query.selection_set.items)
                }
                _ => None,
            })
            .expect("expected a query");

        selections
            .iter()
            .filter_map(|selection| match selection {
                Selection::Field(field) if field.name == "_entities" => Some(field),
                _ => None,
            })
            .map(|field| {
                let Some((_, Value::Variable(variable))) = field.arguments.first() else {
                    panic!("expected representations from a variable");
                };
                let keys = body["variables"][variable.as_str()]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|representation| representation[key].as_str().unwrap().to_string())
                    .collect();
                let mut fields = Vec::new();
                field_names(&field.selection_set.items, &mut fields);
                fields.sort();
                let alias = field.alias.clone().unwrap_or_else(|| field.name.clone());
                (alias, SentAlias { fields, keys })
            })
            .collect()
    }

    /// Runs `mocked_products`, but lets `edit` change what inventory sends back.
    /// `edit` also gets the aliases of the request, so it can point at one of them.
    async fn start_with_inventory_edit(
        edit: impl Fn(&mut serde_json::Value, &BTreeMap<String, SentAlias>) + Send + Sync + 'static,
    ) -> (
        TestSubgraphs<crate::testkit::Started>,
        TestRouter<crate::testkit::Started>,
    ) {
        let mock = crate::testkit::mock_subgraphs::mock_subgraphs(mocked_products());
        let subgraphs = TestSubgraphs::builder()
            .with_on_request(move |request| {
                let aliases = (request.path == "/inventory")
                    .then(|| sent_aliases(request.body.as_deref().unwrap_or_default(), "upc"));
                let mut response = mock(request)?;
                if let Some(aliases) = aliases {
                    let mut json: serde_json::Value =
                        serde_json::from_slice(response.body.as_deref().unwrap()).unwrap();
                    edit(&mut json, &aliases);
                    response.body = Some(json.to_string().into());
                }
                Some(response)
            })
            .build()
            .start()
            .await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                  supergraph:
                    source: file
                    path: supergraph.graphql
                  "#,
            )
            .build()
            .start()
            .await;
        (subgraphs, router)
    }

    /// The alias that selects exactly `fields`.
    fn alias_selecting(aliases: &BTreeMap<String, SentAlias>, fields: &[&str]) -> String {
        aliases
            .iter()
            .find(|(_, alias)| alias.fields == fields)
            .map(|(name, _)| name.clone())
            .unwrap_or_else(|| panic!("no alias selects {fields:?}: {aliases:?}"))
    }

    /// The aliases of the only request inventory got.
    fn only_inventory_request(
        subgraphs: &TestSubgraphs<crate::testkit::Started>,
    ) -> Vec<SentAlias> {
        let requests = subgraphs.get_requests_log("inventory").unwrap_or_default();
        assert_eq!(requests.len(), 1, "expected one inventory request");
        sent_aliases(requests[0].body.as_deref().unwrap(), "upc")
            .into_values()
            .collect()
    }

    #[ntex::test]
    /// `first` and `second` want different fields of the same products, so they get an alias
    /// each. An alias only asks for its own fields, and only for the products at its own paths.
    async fn each_alias_asks_only_for_its_own_fields_and_products() {
        let (subgraphs, router) = start_with_inventory_edit(|_, _| {}).await;

        let res = router
            .send_graphql_request(
                r#"{
                  first: topProducts(first: 2) { upc inStock }
                  second: topProducts(first: 3) { upc shippingEstimate }
                }"#,
                None,
                None,
            )
            .await;

        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
        {
          "data": {
            "first": [
              {
                "upc": "1",
                "inStock": true
              },
              {
                "upc": "2",
                "inStock": false
              }
            ],
            "second": [
              {
                "upc": "1",
                "shippingEstimate": 10
              },
              {
                "upc": "2",
                "shippingEstimate": 40
              },
              {
                "upc": "3",
                "shippingEstimate": 90
              }
            ]
          }
        }
        "#);
        let mut aliases = only_inventory_request(&subgraphs);
        aliases.sort_by(|a, b| a.fields.cmp(&b.fields));
        assert_eq!(
            aliases,
            [
                SentAlias {
                    fields: vec!["inStock".into()],
                    keys: vec!["1".into(), "2".into()],
                },
                SentAlias {
                    fields: vec!["shippingEstimate".into()],
                    keys: vec!["1".into(), "2".into(), "3".into()],
                },
            ]
        );
    }

    #[ntex::test]
    /// `first` and `second` want the same field, so they share one alias. Products `1` and `2`
    /// are in both lists, they're sent once and their result goes to both places.
    async fn product_in_two_lists_is_sent_once_and_lands_in_both() {
        let (subgraphs, router) = start_with_inventory_edit(|_, _| {}).await;

        let res = router
            .send_graphql_request(
                r#"{
                  first: topProducts(first: 2) { upc inStock }
                  second: topProducts(first: 3) { upc inStock }
                }"#,
                None,
                None,
            )
            .await;

        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
        {
          "data": {
            "first": [
              {
                "upc": "1",
                "inStock": true
              },
              {
                "upc": "2",
                "inStock": false
              }
            ],
            "second": [
              {
                "upc": "1",
                "inStock": true
              },
              {
                "upc": "2",
                "inStock": false
              },
              {
                "upc": "3",
                "inStock": true
              }
            ]
          }
        }
        "#);
        assert_eq!(
            only_inventory_request(&subgraphs),
            [SentAlias {
                fields: vec!["inStock".into()],
                keys: vec!["1".into(), "2".into(), "3".into()],
            }]
        );
    }

    #[ntex::test]
    /// Inventory fails `inStock` of product `1`, which is in both lists. The error shows up at
    /// both places, the other products keep their data.
    async fn entity_error_shows_up_wherever_the_entity_is() {
        let (_subgraphs, router) = start_with_inventory_edit(|response, aliases| {
            let alias = alias_selecting(aliases, &["inStock"]);
            response["data"][&alias][0]["inStock"] = serde_json::Value::Null;
            response["errors"] = serde_json::json!([
                { "message": "stock is unknown", "path": [alias, 0, "inStock"] }
            ]);
        })
        .await;

        let res = router
            .send_graphql_request(
                r#"{
                  first: topProducts(first: 2) { upc inStock }
                  second: topProducts(first: 3) { upc inStock }
                }"#,
                None,
                None,
            )
            .await;

        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
        {
          "data": {
            "first": [
              {
                "upc": "1",
                "inStock": null
              },
              {
                "upc": "2",
                "inStock": false
              }
            ],
            "second": [
              {
                "upc": "1",
                "inStock": null
              },
              {
                "upc": "2",
                "inStock": false
              },
              {
                "upc": "3",
                "inStock": true
              }
            ]
          },
          "errors": [
            {
              "message": "Unexpected error",
              "path": [
                "second",
                0,
                "inStock"
              ],
              "extensions": {
                "code": "DOWNSTREAM_SERVICE_ERROR",
                "service": "inventory"
              }
            },
            {
              "message": "Unexpected error",
              "path": [
                "first",
                0,
                "inStock"
              ],
              "extensions": {
                "code": "DOWNSTREAM_SERVICE_ERROR",
                "service": "inventory"
              }
            }
          ]
        }
        "#);
    }

    #[ntex::test]
    /// Inventory fails `shippingEstimate` of product `3`. The error belongs to the alias of
    /// `second`, so it lands in `second` only, and `first` is untouched.
    async fn entity_error_stays_with_its_alias() {
        let (_subgraphs, router) = start_with_inventory_edit(|response, aliases| {
            let alias = alias_selecting(aliases, &["shippingEstimate"]);
            response["data"][&alias][2]["shippingEstimate"] = serde_json::Value::Null;
            response["errors"] = serde_json::json!([
                { "message": "can't ship", "path": [alias, 2, "shippingEstimate"] }
            ]);
        })
        .await;

        let res = router
            .send_graphql_request(
                r#"{
                  first: topProducts(first: 2) { upc inStock }
                  second: topProducts(first: 3) { upc shippingEstimate }
                }"#,
                None,
                None,
            )
            .await;

        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
        {
          "data": {
            "first": [
              {
                "upc": "1",
                "inStock": true
              },
              {
                "upc": "2",
                "inStock": false
              }
            ],
            "second": [
              {
                "upc": "1",
                "shippingEstimate": 10
              },
              {
                "upc": "2",
                "shippingEstimate": 40
              },
              {
                "upc": "3",
                "shippingEstimate": null
              }
            ]
          },
          "errors": [
            {
              "message": "Unexpected error",
              "path": [
                "second",
                2,
                "shippingEstimate"
              ],
              "extensions": {
                "code": "DOWNSTREAM_SERVICE_ERROR",
                "service": "inventory",
                "affectedPath": "second.@"
              }
            }
          ]
        }
        "#);
    }

    #[ntex::test]
    /// An error without a path belongs to no alias. It's reported once, and the data still
    /// lands where it should.
    async fn error_without_a_path_is_reported_once() {
        let (_subgraphs, router) = start_with_inventory_edit(|response, _| {
            response["errors"] = serde_json::json!([{ "message": "something is off" }]);
        })
        .await;

        let res = router
            .send_graphql_request(
                r#"{
                  first: topProducts(first: 2) { upc inStock }
                  second: topProducts(first: 3) { upc shippingEstimate }
                }"#,
                None,
                None,
            )
            .await;

        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
        {
          "data": {
            "first": [
              {
                "upc": "1",
                "inStock": true
              },
              {
                "upc": "2",
                "inStock": false
              }
            ],
            "second": [
              {
                "upc": "1",
                "shippingEstimate": 10
              },
              {
                "upc": "2",
                "shippingEstimate": 40
              },
              {
                "upc": "3",
                "shippingEstimate": 90
              }
            ]
          },
          "errors": [
            {
              "message": "Unexpected error",
              "extensions": {
                "code": "DOWNSTREAM_SERVICE_ERROR",
                "service": "inventory"
              }
            }
          ]
        }
        "#);
    }

    #[ntex::test]
    /// Inventory can't find product `2` and returns `null` for it. Only product `2` loses its
    /// fields, products `1` and `3` still get theirs, in both aliases.
    async fn missing_entity_does_not_shift_the_others() {
        let (_subgraphs, router) = start_with_inventory_edit(|response, aliases| {
            for alias in aliases.keys() {
                response["data"][alias][1] = serde_json::Value::Null;
            }
        })
        .await;

        let res = router
            .send_graphql_request(
                r#"{
                  first: topProducts(first: 2) { upc inStock }
                  second: topProducts(first: 3) { upc shippingEstimate }
                }"#,
                None,
                None,
            )
            .await;

        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
        {
          "data": {
            "first": [
              {
                "upc": "1",
                "inStock": true
              },
              {
                "upc": "2",
                "inStock": null
              }
            ],
            "second": [
              {
                "upc": "1",
                "shippingEstimate": 10
              },
              {
                "upc": "2",
                "shippingEstimate": null
              },
              {
                "upc": "3",
                "shippingEstimate": 90
              }
            ]
          }
        }
        "#);
    }

    #[ntex::test]
    /// The whole batch fails. Every batched field is `null`, there's an error for it, and the
    /// data from `products` stays.
    async fn failed_batch_keeps_the_rest_of_the_response() {
        let (_subgraphs, router) = start_with_inventory_edit(|response, _| {
            *response = serde_json::json!({
                "data": null,
                "errors": [{ "message": "inventory is down" }]
            });
        })
        .await;

        let res = router
            .send_graphql_request(
                r#"{
                  first: topProducts(first: 2) { upc inStock }
                  second: topProducts(first: 3) { upc shippingEstimate }
                }"#,
                None,
                None,
            )
            .await;

        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
        {
          "data": {
            "first": [
              {
                "upc": "1",
                "inStock": null
              },
              {
                "upc": "2",
                "inStock": null
              }
            ],
            "second": [
              {
                "upc": "1",
                "shippingEstimate": null
              },
              {
                "upc": "2",
                "shippingEstimate": null
              },
              {
                "upc": "3",
                "shippingEstimate": null
              }
            ]
          },
          "errors": [
            {
              "message": "Unexpected error",
              "extensions": {
                "code": "DOWNSTREAM_SERVICE_ERROR",
                "service": "inventory"
              }
            }
          ]
        }
        "#);
    }

    #[ntex::test]
    /// https://github.com/graphql-hive/router/issues/1732
    ///
    /// Each union member wants other fields of its `Product`. The products under `ComponentA`
    /// are only asked for `a`, the one under `ComponentB` only for `b`. There's no `ComponentC`,
    /// so nothing is asked for `c`. All of it goes out in one request.
    async fn union_members_ask_only_for_their_own_entity_fields() {
        use crate::testkit::mock_subgraphs::mock_subgraphs;

        let component = |typename: &str, id: &str| serde_json::json!({ "__typename": typename, "product": { "__typename": "Product", "id": id } });
        let subgraphs = TestSubgraphs::builder()
            .with_on_request(mock_subgraphs(serde_json::json!({
                "card": {
                    "query": {
                        "card": {
                            "components": [
                                component("ComponentA", "p1"),
                                component("ComponentB", "p2"),
                                component("ComponentA", "p3")
                            ]
                        }
                    }
                },
                "product": {
                    "entities": [
                        { "__typename": "Product", "id": "p1", "a": "a1" },
                        { "__typename": "Product", "id": "p2", "b": "b2" },
                        { "__typename": "Product", "id": "p3", "a": "a3" }
                    ]
                }
            })))
            .build()
            .start()
            .await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                  supergraph:
                    source: file
                    path: ../bin/router/fixture/tests/union-member-entity-fields.supergraph.graphql
                  "#,
            )
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(
                r#"{
                  card {
                    components {
                      __typename
                      ... on ComponentA { product { id a } }
                      ... on ComponentB { product { id b } }
                      ... on ComponentC { product { id c } }
                    }
                  }
                }"#,
                None,
                None,
            )
            .await;

        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
        {
          "data": {
            "card": {
              "components": [
                {
                  "__typename": "ComponentA",
                  "product": {
                    "id": "p1",
                    "a": "a1"
                  }
                },
                {
                  "__typename": "ComponentB",
                  "product": {
                    "id": "p2",
                    "b": "b2"
                  }
                },
                {
                  "__typename": "ComponentA",
                  "product": {
                    "id": "p3",
                    "a": "a3"
                  }
                }
              ]
            }
          }
        }
        "#);

        let requests = subgraphs.get_requests_log("product").unwrap_or_default();
        assert_eq!(requests.len(), 1, "expected one product request");
        let mut aliases: Vec<SentAlias> = sent_aliases(requests[0].body.as_deref().unwrap(), "id")
            .into_values()
            .collect();
        aliases.sort_by(|a, b| a.fields.cmp(&b.fields));
        assert_eq!(
            aliases,
            [
                SentAlias {
                    fields: vec!["a".into()],
                    keys: vec!["p1".into(), "p3".into()],
                },
                SentAlias {
                    fields: vec!["b".into()],
                    keys: vec!["p2".into()],
                },
                SentAlias {
                    fields: vec!["c".into()],
                    keys: vec![],
                },
            ]
        );
    }

    #[ntex::test]
    /// https://github.com/graphql-hive/router/issues/1733
    ///
    /// `details` and `inventory` both come from `b`, each under its own condition. `b` is only
    /// called when one of them is on, and the product is sent once even with both on.
    ///
    /// ```text
    /// $skipDetails  $withInventory  requests to b  aliases that run
    /// false         true            1              details, inventory
    /// true          true            1              inventory
    /// false         false           1              details
    /// true          false           0
    /// ```
    async fn entity_fields_under_different_conditions_skip_the_request_when_all_are_off() {
        use crate::testkit::mock_subgraphs::mock_subgraphs;

        let subgraphs = TestSubgraphs::builder()
            .with_on_request(mock_subgraphs(serde_json::json!({
                "a": {
                    "query": { "product": { "__typename": "Product", "id": "p1" } }
                },
                "b": {
                    "entities": [{
                        "__typename": "Product",
                        "id": "p1",
                        "details": { "color": "red" },
                        "inventory": { "inStock": true }
                    }]
                }
            })))
            .build()
            .start()
            .await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                  supergraph:
                    source: file
                    path: ../bin/router/fixture/tests/skip-entity-fetch.supergraph.graphql
                  "#,
            )
            .build()
            .start()
            .await;

        let query = r#"
          query ($skipDetails: Boolean!, $withInventory: Boolean!) {
            product {
              id
              details @skip(if: $skipDetails) { color }
              inventory @include(if: $withInventory) { inStock }
            }
          }
        "#;

        let cases = [(false, true), (true, true), (false, false), (true, false)];
        let mut sent = 0;
        for (skip_details, with_inventory) in cases {
            let res = router
                .send_graphql_request(
                    query,
                    Some(sonic_rs::json!({
                        "skipDetails": skip_details,
                        "withInventory": with_inventory
                    })),
                    None,
                )
                .await;
            let json_body = res.json_body().await;
            let case = format!("skipDetails={skip_details} withInventory={with_inventory}");
            assert!(json_body["errors"].is_null(), "{case}: {json_body}");
            assert_eq!(
                json_body
                    .pointer(&pointer!["data", "product", "details", "color"])
                    .as_str(),
                (!skip_details).then_some("red"),
                "{case}: {json_body}"
            );
            assert_eq!(
                json_body
                    .pointer(&pointer!["data", "product", "inventory", "inStock"])
                    .as_bool(),
                with_inventory.then_some(true),
                "{case}: {json_body}"
            );

            let requests = subgraphs.get_requests_log("b").unwrap_or_default();
            if skip_details && !with_inventory {
                assert_eq!(requests.len(), sent, "{case}: no request to b expected");
                continue;
            }
            sent += 1;
            assert_eq!(requests.len(), sent, "{case}: one request to b expected");

            let body: serde_json::Value =
                serde_json::from_slice(requests[sent - 1].body.as_deref().unwrap()).unwrap();
            let variables = body["variables"].as_object().unwrap();
            let representations: Vec<&serde_json::Value> = variables
                .iter()
                .filter(|(name, _)| name.starts_with("__batch_reps"))
                .map(|(_, value)| value)
                .collect();
            assert_eq!(
                representations,
                [&serde_json::json!([{ "__typename": "Product", "id": "p1" }])],
                "{case}: the product is sent once"
            );
            let mut runs: Vec<bool> = variables
                .iter()
                .filter(|(name, _)| name.starts_with("__batch_run"))
                .map(|(_, value)| value.as_bool().unwrap())
                .collect();
            runs.sort();
            let mut expected = vec![!skip_details, with_inventory];
            expected.sort();
            assert_eq!(runs, expected, "{case}: which aliases run");
        }
    }
}
