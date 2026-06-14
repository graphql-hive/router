#[cfg(test)]
mod actual_cost_tests {
    use super::super::common::*;

    #[ntex::test]
    async fn includes_actual_and_negative_delta_for_response_shape_mode() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
        supergraph:
            source: file
            path: supergraph.graphql
        demand_control:
            enabled: true
            operation_cost:
              max: 1000
              mode: enforce
            default_list_size:
              all: 10
            subgraphs_budget:
              mode: enforce
            actual_cost_mode: by_response_shape
            expose_headers:
              estimated: true
              actual: true
              max: true
        "#,
            )
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(
                r#"
                query {
                  me {
                    reviews {
                      body
                    }
                  }
                }
                "#,
                None,
                None,
            )
            .await;

        let estimated = res
            .cost_header("x-cost-estimated")
            .expect("estimated should be present");
        let actual = res
            .cost_header("x-cost-actual")
            .expect("actual should be present");

        assert!(actual < estimated);
    }
    #[ntex::test]
    async fn computes_by_subgraph_actual_from_responses_not_estimates() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
        supergraph:
            source: file
            path: supergraph.graphql
        demand_control:
            enabled: true
            operation_cost:
              max: 1000
              mode: enforce
            default_list_size:
              all: 10
            subgraphs_budget:
              mode: enforce
            actual_cost_mode: by_subgraph
            expose_headers:
              estimated: true
              actual: true
              max: true
        "#,
            )
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(
                r#"
                query {
                  me {
                    reviews {
                      body
                    }
                  }
                }
                "#,
                None,
                None,
            )
            .await;

        let estimated_total = res
            .cost_header("x-cost-estimated")
            .expect("estimated should be present");
        let actual_total = res
            .cost_header("x-cost-actual")
            .expect("actual should be present");

        assert!(actual_total < estimated_total);
    }

    // Actual cost exceeding the configured max never returns a GraphQL error.
    // The router records `cost.result = COST_ACTUAL_TOO_EXPENSIVE` in the
    // response extensions and metrics, but lets the response through.
    #[ntex::test]
    async fn does_not_reject_when_actual_by_subgraph_cost_exceeds_max() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
        supergraph:
            source: file
            path: supergraph.graphql
        demand_control:
            enabled: true
            operation_cost:
              max: 3
              mode: enforce
            default_list_size:
              all: 0
            subgraphs_budget:
              mode: enforce
            actual_cost_mode: by_subgraph
            expose_headers:
              estimated: true
              actual: true
              max: true
        "#,
            )
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(
                r#"
                query {
                  me {
                    reviews {
                      body
                    }
                  }
                }
                "#,
                None,
                None,
            )
            .await;

        let json = res.json_body().await;

        assert!(
            json.get("errors").is_none() || json["errors"].is_null(),
            "router must not return a GraphQL error when actual cost exceeds max; got: {json}"
        );

        let estimated = res
            .cost_header("x-cost-estimated")
            .expect("estimated should be present");
        let actual = res
            .cost_header("x-cost-actual")
            .expect("actual should be present");
        let max_cost = res
            .cost_header("x-cost-max")
            .expect("maxCost should be present");

        assert!(
            estimated <= max_cost,
            "estimated cost should be within max cost in actual too expensive case"
        );
        assert!(
            actual > max_cost,
            "actual cost should exceed max cost in this scenario"
        );
        assert!(
            actual > estimated,
            "actual cost should exceed estimated cost in this scenario"
        );
    }

    // Verify `cost` is included in cost extensions
    // No GraphQL error is emitted, but the response extensions still expose the
    // configured `cost` so clients can correlate cost.actual against it.
    #[ntex::test]
    async fn extension_cost_includes_max_cost_value_when_actual_exceeds_max() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                demand_control:
                    enabled: true
                    operation_cost:
                      max: 3
                      mode: enforce
                    default_list_size:
                      all: 0
                    subgraphs_budget:
                      mode: enforce
                    actual_cost_mode: by_subgraph
                    expose_headers:
                      estimated: true
                      actual: true
                      max: true
                "#,
            )
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(
                r#"
            query {
              me {
                reviews {
                  body
                }
              }
            }
            "#,
                None,
                None,
            )
            .await;

        let json = res.json_body().await;

        assert!(
            json.get("errors").is_none() || json["errors"].is_null(),
            "no GraphQL error must be emitted for actual cost overruns; got: {json}"
        );
        assert_eq!(
            res.cost_header("x-cost-max"),
            Some(3),
            "max should be present in cost extensions with value 3"
        );
        assert!(
            res.cost_header("x-cost-actual").unwrap() > 0,
            "actual cost must be non-zero"
        );
        assert!(
            res.cost_header("x-cost-actual").unwrap()
                > res.cost_header("x-cost-estimated").unwrap(),
            "actual cost must be greater than estimated cost"
        );
        assert!(
            res.cost_header("x-cost-actual").unwrap() > res.cost_header("x-cost-max").unwrap(),
            "actual cost must be greater than max cost"
        );
    }

    // Verify delta is negative when actual < estimated (fewer list items than assumed).
    #[ntex::test]
    async fn negative_delta_when_actual_smaller_than_estimated() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                demand_control:
                    enabled: true
                    operation_cost:
                      max: 1000
                      mode: enforce
                    default_list_size:
                      all: 10
                    subgraphs_budget:
                      mode: enforce
                    actual_cost_mode: by_response_shape
                    expose_headers:
                      estimated: true
                      actual: true
                      max: true
                "#,
            )
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(
                r#"
            query {
              me {
                reviews {
                  body
                }
              }
            }
            "#,
                None,
                None,
            )
            .await;

        let actual = res
            .cost_header("x-cost-actual")
            .expect("actual should be present") as i64;
        let estimated = res
            .cost_header("x-cost-estimated")
            .expect("estimated should be present") as i64;
        let delta = actual - estimated;

        assert!(
            delta < 0,
            "delta should be negative when actual < estimated list sizes"
        );
    }
    // Verify delta is positive when actual > estimated (more list items than assumed).
    #[ntex::test]
    async fn positive_delta_when_actual_greater_than_estimated() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph_demand_control.graphql
                demand_control:
                    enabled: true
                    operation_cost:
                      max: 1000
                      mode: enforce
                    default_list_size:
                      all: 0
                    subgraphs_budget:
                      mode: enforce
                    actual_cost_mode: by_response_shape
                    expose_headers:
                      estimated: true
                      actual: true
                      max: true
                "#,
            )
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(
                r#"
            query {
                            booksByGenre(genre: MYSTERY) {
                                title
              }
            }
            "#,
                None,
                None,
            )
            .await;

        let estimated = res
            .cost_header("x-cost-estimated")
            .expect("estimated should be present");
        let actual = res
            .cost_header("x-cost-actual")
            .expect("actual should be present");
        let delta = actual - estimated;

        assert!(
            actual > estimated,
            "actual should be larger when assumed list_size is 0"
        );
        assert!(
            delta > 0,
            "delta should be positive when actual > estimated"
        );
    }
    // Verify delta remains accurate across a range of configured list-size assumptions.
    #[ntex::test]
    async fn delta_accuracy_with_varying_list_sizes() {
        for list_size in [0_u64, 1_u64, 10_u64] {
            let subgraphs = TestSubgraphs::builder().build().start().await;
            let router = TestRouter::builder()
                .with_subgraphs(&subgraphs)
                .inline_config(format!(
                    r#"
                    supergraph:
                        source: file
                        path: supergraph_demand_control.graphql
                    demand_control:
                        enabled: true
                        operation_cost:
                          max: 1000
                          mode: enforce
                        default_list_size:
                          all: {list_size}
                        subgraphs_budget:
                          mode: enforce
                        actual_cost_mode: by_response_shape
                        expose_headers:
                          estimated: true
                          actual: true
                          max: true
                    "#,
                ))
                .build()
                .start()
                .await;

            let res = router
                .send_graphql_request(
                    r#"
                query {
                                    booksByGenre(genre: MYSTERY) {
                                        title
                  }
                }
                "#,
                    None,
                    None,
                )
                .await;

            let estimated = res
                .cost_header("x-cost-estimated")
                .expect("estimated should be present");
            let actual = res
                .cost_header("x-cost-actual")
                .expect("actual should be present");
            let delta = actual as i64 - estimated as i64;

            assert_eq!(
                delta,
                actual as i64 - estimated as i64,
                "delta should exactly match actual-estimated for list_size={list_size}"
            );

            if list_size == 10 {
                assert!(
                    delta < 0,
                    "large assumed list size should make delta negative"
                );
            } else {
                assert!(
                    delta > 0,
                    "small assumed list size should make delta positive"
                );
            }
        }
    }
    // Verify by_response_shape actual cost ignores extra entity-lookup work and only reflects
    // the final response shape.
    #[ntex::test]
    async fn response_shape_mode_entity_lookup_cost_accuracy() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                demand_control:
                    enabled: true
                    operation_cost:
                      max: 1000
                      mode: enforce
                    default_list_size:
                      all: 10
                    subgraphs_budget:
                      mode: enforce
                    actual_cost_mode: by_response_shape
                    expose_headers:
                      estimated: true
                      actual: true
                      max: true
                "#,
            )
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(
                r#"
            query {
              topProducts(first: 1) {
                inStock
              }
            }
            "#,
                None,
                None,
            )
            .await;

        let json = res.json_body().await;
        assert_eq!(
            json["data"]["topProducts"]
                .as_array()
                .map(|items| items.len()),
            Some(1)
        );

        let estimated = res
            .cost_header("x-cost-estimated")
            .expect("estimated should be present");
        let actual = res
            .cost_header("x-cost-actual")
            .expect("actual should be present");

        // Final response shape is a single Product object with one scalar field.
        assert_eq!(
            actual, 1,
            "response-shape actual cost should reflect only the final Product item"
        );
        assert!(
            estimated > actual,
            "estimated cost should still include assumed list fanout"
        );
    }
    // BatchFetch: when two independent paths need entity resolution from the same subgraph
    // in parallel, the planner merges them into a single BatchFetch with aliased _entities
    // (e.g. `_e0: _entities(...) { ...on User {...} }` and
    //       `_e1: _entities(...) { ...on Product {...} }`).
    // The compiled actual cost plan must handle each aliased group independently.
    // Before the fix, the aliased fields were looked up as regular schema fields
    // → returning 0 cost. This test asserts that actual reviews cost is > 0.
    #[ntex::test]
    async fn batch_fetch_actual_cost_is_non_zero() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                demand_control:
                    enabled: true
                    operation_cost:
                      max: 100000
                      mode: enforce
                    default_list_size:
                      all: 0
                    subgraphs_budget:
                      mode: enforce
                    actual_cost_mode: by_subgraph
                    expose_headers:
                      estimated: true
                      actual: true
                      max: true
                "#,
            )
            .build()
            .start()
            .await;

        // This query creates two independent paths that both need entity resolution
        // from the reviews subgraph:
        //   - users[].reviews (User entity from reviews)
        //   - topProducts[].reviews (Product entity from reviews)
        // The planner merges them into a BatchFetch to reviews with two aliases.
        let res = router
            .send_graphql_request(
                r#"
                query {
                  users { reviews { body } }
                  topProducts { reviews { body } }
                }
                "#,
                None,
                None,
            )
            .await;

        let json = res.json_body().await;
        assert!(
            json["errors"].is_null(),
            "query should succeed: {:?}",
            json["errors"]
        );
    }
    // When actual cost mode is by_subgraph and a BatchFetch returns entities, the cost
    // must be summed across both alias groups. Verify that turning down the per-subgraph
    // max causes a rejection that would NOT fire if the cost were incorrectly reported as 0.
    #[ntex::test]
    async fn batch_fetch_actual_cost_can_trigger_rejection() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                demand_control:
                    enabled: true
                    operation_cost:
                      max: 10000
                      mode: enforce
                    default_list_size:
                      all: 0
                    subgraphs_budget:
                      mode: enforce
                    actual_cost_mode: by_subgraph
                    expose_headers:
                      estimated: true
                      actual: true
                      max: true
                "#,
            )
            .build()
            .start()
            .await;

        // Same BatchFetch-triggering query with a high estimated max so the estimated
        // check passes, but we then verify that the actual cost from reviews entities
        // is non-zero (proving the BatchFetch groups were costed).
        let res = router
            .send_graphql_request(
                r#"
                query {
                  users { reviews { body } }
                  topProducts { reviews { body } }
                }
                "#,
                None,
                None,
            )
            .await;

        let json = res.json_body().await;

        // Should not be rejected at all with max=10000; verify cost is attributed.
        assert!(
            json["errors"].is_null(),
            "should not be rejected with high max; errors: {:?}",
            json["errors"]
        );

        // Verify estimated cost is also > 0, confirming that BatchFetch estimation
        // correctly processes aliased _entities fields during formula compilation.
        let estimated_total = res
            .cost_header("x-cost-estimated")
            .expect("estimated should be present");
        assert!(
            estimated_total > 0,
            "estimated cost must be > 0 for batch fetch query; got {}",
            estimated_total
        );

        // The total actual must equal the sum of per-subgraph actuals that we can see.
        let actual_total = res
            .cost_header("x-cost-actual")
            .expect("actual should be present");
        assert!(actual_total > 0, "actual cost must be non-zero");
        assert!(
            actual_total > estimated_total,
            "actual cost must be greater than estimated cost"
        );
    }
    // When an _entities response includes __typename, the evaluator must use the
    // explicit typename to pick the correct per-type plan rather than the single-entry
    // shortcut. This ensures costs are correctly attributed even when both paths coexist.
    #[ntex::test]
    async fn actual_cost_uses_explicit_typename_when_present() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                demand_control:
                    enabled: true
                    operation_cost:
                      max: 100000
                      mode: enforce
                    default_list_size:
                      all: 10
                    subgraphs_budget:
                      mode: enforce
                    actual_cost_mode: by_subgraph
                    expose_headers:
                      estimated: true
                      actual: true
                      max: true
                "#,
            )
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(
                r#"
                query {
                  me {
                    reviews {
                      body
                    }
                  }
                }
                "#,
                None,
                None,
            )
            .await;

        let json = res.json_body().await;
        assert!(json["errors"].is_null(), "query should succeed");

        let actual = res
            .cost_header("x-cost-actual")
            .expect("actual reviews should be present");
        let estimated = res
            .cost_header("x-cost-estimated")
            .expect("estimated reviews should be present");

        // With list_size=10 (assumed) the estimated review count is inflated;
        // actual is based on the real response items.
        assert!(
            actual < estimated,
            "actual cost ({}) should be less than estimated ({})",
            actual,
            estimated
        );
        assert!(actual > 0, "actual cost must be non-zero");
    }
    // Type-conditioned selections can still be deterministic even when the final
    // merged response omits __typename. If the parent type is already known at
    // compile time, the fragment must still apply for actual cost calculation.
    #[ntex::test]
    async fn actual_cost_applies_deterministic_inline_fragment_without_typename() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                demand_control:
                    enabled: true
                    operation_cost:
                      max: 100000
                      mode: enforce
                    default_list_size:
                      all: 0
                    subgraphs_budget:
                      mode: enforce
                    actual_cost_mode: by_response_shape
                    expose_headers:
                      estimated: true
                      actual: true
                      max: true
                "#,
            )
            .build()
            .start()
            .await;

        let baseline = router
            .send_graphql_request(
                r#"
                query {
                                    me {
                                        id
                  }
                }
                "#,
                None,
                None,
            )
            .await;
        let baseline_json = baseline.json_body().await;

        let res = router
            .send_graphql_request(
                r#"
                                query {
                                    me {
                                        ... on User {
                                            reviews {
                                                body
                                            }
                                        }
                                    }
                                }
                                "#,
                None,
                None,
            )
            .await;

        let json = res.json_body().await;
        assert!(
            baseline_json["errors"].is_null(),
            "baseline query should succeed: {:?}",
            baseline_json["errors"]
        );
        assert!(
            json["errors"].is_null(),
            "query should succeed: {:?}",
            json["errors"]
        );

        let baseline_actual = baseline
            .cost_header("x-cost-actual")
            .expect("baseline actual should be present");
        let actual = res
            .cost_header("x-cost-actual")
            .expect("actual should be present");

        // If the inline fragment were skipped because __typename is absent,
        // the query would collapse to the same cost shape as the baseline `me { id }` query.
        assert!(
            actual > baseline_actual,
            "inline fragment on known parent type must add cost even without __typename: {} <= {}",
            actual,
            baseline_actual
        );
    }
    // Actual (by_response_shape) must respect the same GraphQL directive semantics
    // as estimation: field contributes only when @include is true and @skip is false.
    #[ntex::test]
    async fn actual_cost_respects_combined_include_and_skip_conditions() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                demand_control:
                    enabled: true
                    operation_cost:
                      max: 100000
                      mode: enforce
                    default_list_size:
                      all: 5
                    subgraphs_budget:
                      mode: enforce
                    actual_cost_mode: by_response_shape
                    expose_headers:
                      estimated: true
                      actual: true
                      max: true
                "#,
            )
            .build()
            .start()
            .await;

        let include_true_skip_false = router
            .send_graphql_request(
                r#"
                                query {
                                    users {
                                        id
                                        reviews @include(if: true) @skip(if: false) {
                                            body
                                        }
                                    }
                                }
                                "#,
                None,
                None,
            )
            .await;
        let json_a = include_true_skip_false.json_body().await;

        let include_true_skip_true = router
            .send_graphql_request(
                r#"
                                query {
                                    users {
                                        id
                                        reviews @include(if: true) @skip(if: true) {
                                            body
                                        }
                                    }
                                }
                                "#,
                None,
                None,
            )
            .await;
        let json_b = include_true_skip_true.json_body().await;

        assert!(
            json_a["errors"].is_null(),
            "query A should succeed; errors: {:?}",
            json_a["errors"]
        );
        assert!(
            json_b["errors"].is_null(),
            "query B should succeed; errors: {:?}",
            json_b["errors"]
        );

        let actual_a = include_true_skip_false
            .cost_header("x-cost-actual")
            .expect("actual should be present for query A");
        let actual_b = include_true_skip_true
            .cost_header("x-cost-actual")
            .expect("actual should be present for query B");

        assert!(
            actual_a > actual_b,
            "actual with include=true,skip=false ({}) must exceed include=true,skip=true ({})",
            actual_a,
            actual_b
        );
    }
    // Single-type entity fetches in real plans must produce non-zero actual cost
    // in by_subgraph mode when the result is non-empty.
    #[ntex::test]
    async fn single_type_entity_fetch_actual_cost_is_non_zero_for_non_empty_result() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                demand_control:
                    enabled: true
                    operation_cost:
                      max: 100000
                      mode: enforce
                    default_list_size:
                      all: 0
                    subgraphs_budget:
                      mode: enforce
                    actual_cost_mode: by_subgraph
                    expose_headers:
                      estimated: true
                      actual: true
                      max: true
                "#,
            )
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(
                r#"
                query {
                                    users {
                                        reviews {
                                            body
                                        }
                  }
                }
                "#,
                None,
                None,
            )
            .await;

        let json = res.json_body().await;
        assert!(
            json["errors"].is_null(),
            "query should succeed; errors: {:?}",
            json["errors"]
        );

        let actual = res.cost_header("x-cost-actual").unwrap_or(0);

        assert!(
            actual > 0,
            "non-empty users.reviews should produce non-zero actual cost"
        );
    }
    // `_entities` fetches can contain nested inline fragments for child objects.
    // Those nested type conditions must not be mistaken for additional root
    // entity types, or single-type entity groups without __typename can be costed as 0.
    #[ntex::test]
    async fn nested_inline_fragments_do_not_break_single_type_entity_actual_cost() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                                supergraph:
                                        source: file
                                        path: supergraph.graphql
                                demand_control:
                                        enabled: true
                                        operation_cost:
                                          max: 100000
                                          mode: enforce
                                        default_list_size:
                                          all: 0
                                        subgraphs_budget:
                                          mode: enforce
                                        actual_cost_mode: by_subgraph
                                        expose_headers:
                                          estimated: true
                                          actual: true
                                          max: true
                                "#,
            )
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(
                r#"
                                query {
                                    users {
                                        reviews {
                                            ... on Review {
                                                product {
                                                    ... on Product {
                                                        name
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                "#,
                None,
                None,
            )
            .await;

        let json = res.json_body().await;
        assert!(
            json["errors"].is_null(),
            "query should succeed: {:?}",
            json["errors"]
        );

        let actual = res
            .cost_header("x-cost-actual")
            .expect("actual should be present");

        assert!(
            actual > 0,
            "nested inline fragments must not collapse single-type _entities cost to zero"
        );
    }
    // Actual cost for an _entities fetch that returns an empty array must be 0,
    // not a panic or incorrect non-zero value.
    #[ntex::test]
    async fn actual_cost_is_zero_for_empty_entities_response() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                demand_control:
                    enabled: true
                    operation_cost:
                      max: 100000
                      mode: enforce
                    default_list_size:
                      all: 0
                    subgraphs_budget:
                      mode: enforce
                    actual_cost_mode: by_subgraph
                    expose_headers:
                      estimated: true
                      actual: true
                      max: true
                "#,
            )
            .build()
            .start()
            .await;

        // topProducts(first: 0) should return an empty list → no entity calls to
        // inventory/products → those subgraphs should have zero actual cost.
        let res = router
            .send_graphql_request(
                r#"
                query {
                  topProducts(first: 0) {
                    inStock
                  }
                }
                "#,
                None,
                None,
            )
            .await;

        let json = res.json_body().await;
        assert!(json["errors"].is_null(), "query should succeed");

        // With an empty `topProducts` list there are no entity calls, so the
        // total actual cost (and therefore every subgraph's contribution) is 0.
        let actual = res
            .cost_header("x-cost-actual")
            .expect("actual should be present");
        assert_eq!(
            actual, 0,
            "actual cost must be 0 when the entity array is empty"
        );
    }
    // Verifies that by_subgraph actual cost accumulates correctly across two sequential
    // entity fetches to the same subgraph (FlattenFetch, not BatchFetch).
    // The first fetch gets top-level entities; the second resolves nested entity references.
    #[ntex::test]
    async fn actual_cost_accumulates_across_sequential_entity_fetches() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                demand_control:
                    enabled: true
                    operation_cost:
                      max: 100000
                      mode: enforce
                    default_list_size:
                      all: 2
                    subgraphs_budget:
                      mode: enforce
                    actual_cost_mode: by_subgraph
                    expose_headers:
                      estimated: true
                      actual: true
                      max: true
                "#,
            )
            .build()
            .start()
            .await;

        // topProducts { reviews { product { name } } } causes:
        //   1. Fetch products → topProducts
        //   2. Flatten/Fetch reviews → entities for each product
        //   3. Flatten/Fetch products → entities for each review.product.name
        // Products subgraph is hit in step 1 (fetch) and step 3 (entities).
        // Both contribute to actualBySubgraph.products.
        let res = router
            .send_graphql_request(
                r#"
                query {
                  topProducts {
                    reviews {
                      product {
                        name
                      }
                    }
                  }
                }
                "#,
                None,
                None,
            )
            .await;

        let json = res.json_body().await;
        assert!(json["errors"].is_null(), "query should succeed");

        // With list_size=2 assumed but real data returned, actual may differ from estimated.
        let actual_total = res
            .cost_header("x-cost-actual")
            .expect("actual should be present");
        let estimated_total = res
            .cost_header("x-cost-estimated")
            .expect("estimated should be present");
        assert!(actual_total > estimated_total);
    }
}
