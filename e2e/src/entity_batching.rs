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
}
