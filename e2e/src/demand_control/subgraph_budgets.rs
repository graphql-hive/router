#[cfg(test)]
mod subgraph_budgets_tests {
    use super::super::common::*;

    #[ntex::test]
    async fn skips_only_over_budget_subgraph_and_continues_query() {
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
            mode: enforce
            strategy:
              static_estimated:
                max: 1000
                subgraph:
                  subgraphs:
                    reviews:
                      max: 0
                  all:
                    list_size: 0
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
                    name
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

        // Cost is exposed via the `X-Cost-*` headers (no longer in response extensions).
        assert_eq!(res.cost_header("x-cost-estimated"), Some(2));
        assert_eq!(res.cost_header("x-cost-actual"), Some(1));
        assert_eq!(res.cost_header("x-cost-max"), Some(1000));

        let json = res.json_body().await;

        // The over-budget `reviews` subgraph is skipped; the rest of the plan runs.
        assert_eq!(json["data"]["me"]["name"].as_str(), Some("Uri Goldshtein"));
        assert!(
            json["data"]["me"]["reviews"].is_null(),
            "reviews should be null after the subgraph was skipped: {json}"
        );

        // The subgraph-skip error keeps its `cost` extension.
        let err = &json["errors"][0];
        assert_eq!(
            err["message"].as_str(),
            Some("Skipped subgraph execution because the estimated cost (1) exceeds the maximum allowed cost (0).")
        );
        assert_eq!(
            err["extensions"]["code"].as_str(),
            Some("SUBGRAPH_COST_ESTIMATED_TOO_EXPENSIVE")
        );
        assert_eq!(err["extensions"]["serviceName"].as_str(), Some("reviews"));
        assert_eq!(err["extensions"]["affectedPath"].as_str(), Some("me"));
        assert_eq!(err["extensions"]["cost"]["estimated"].as_u64(), Some(1));
        assert_eq!(err["extensions"]["cost"]["max"].as_u64(), Some(0));

        // Cost is no longer duplicated in the top-level response extensions.
        assert!(
            json["extensions"]["cost"].is_null(),
            "top-level cost extension must be gone: {json}"
        );
    }
    // Per-subgraph max_cost setting allows granular control.
    #[ntex::test]
    async fn per_subgraph_max_cost_limit_enforced() {
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
                    mode: enforce
                    strategy:
                      static_estimated:
                        max: 10000
                        subgraph:
                            subgraphs: {}
                            all:
                                list_size: 5
                                max: 1
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
        // Should have subgraph error for per-subgraph cost limit
        assert_eq!(
            json["errors"][0]["extensions"]["code"].as_str(),
            Some("SUBGRAPH_COST_ESTIMATED_TOO_EXPENSIVE")
        );
    }
    // Named subgraph max_cost override (higher) allows queries that all.max_cost would reject.
    // accounts has specific max_cost=100; products inherits all.max_cost=1.
    // Combined query hits both subgraphs only products (over-budget) is blocked.
    #[ntex::test]
    async fn named_subgraph_max_allows_where_all_would_reject() {
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
                        mode: enforce
                        strategy:
                          static_estimated:
                            list_size: 2
                            max: 1000
                            subgraph:
                                subgraphs:
                                    accounts:
                                        max: 100
                                all:
                                    max: 1
                        expose_headers:
                          estimated: true
                          actual: true
                          max: true
                    "#,
            )
            .build()
            .start()
            .await;

        // users comes from accounts (max=100), topProducts from products (inherits all.max=1).
        // With list_size=2: users costs 2*(User=1)=2 < 100 → passes.
        // topProducts costs 2*(Product=1)=2 > 1 → blocked.
        let res = router
            .send_graphql_request(r#"{ users { name } topProducts { name } }"#, None, None)
            .await;

        let json = res.json_body().await;

        // accounts (users) succeeds because named override gives it max_cost=100
        assert!(
            json["data"]["users"].is_array(),
            "users should be returned from accounts (named override allows it)"
        );
        // products is blocked because it inherits all.max_cost=1 (no named override)
        let blocked_products = json["errors"].as_array().map_or(false, |errors| {
            errors.iter().any(|e| {
                e["extensions"]["code"].as_str() == Some("SUBGRAPH_COST_ESTIMATED_TOO_EXPENSIVE")
                    && e["extensions"]["serviceName"].as_str() == Some("products")
            })
        });
        assert!(
            blocked_products,
            "products should be blocked (inherits all.max_cost=1, cost=2 exceeds it)"
        );
    }
    // Per-subgraph aggregate cost sums costs across ALL fetches to the same subgraph.
    // The query causes PRODUCTS to be fetched twice in the query plan:
    //   1. topProducts { upc } (initial fetch, cost=2 with list_size=2)
    //   2. _entities for product.name referenced from reviews (entity lookup, adds to aggregate)
    // Each individual fetch is within the per-subgraph limit, but the aggregate exceeds it.
    #[ntex::test]
    async fn per_subgraph_aggregate_cost_across_multiple_fetches() {
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
                        mode: enforce
                        strategy:
                          static_estimated:
                            list_size: 2
                            max: 1000
                            subgraph:
                                subgraphs:
                                    products:
                                        max: 2
                                all:
                                    max: 1000
                        expose_headers:
                          estimated: true
                          actual: true
                          max: true
                    "#,
            )
            .build()
            .start()
            .await;

        // Query plan forces two PRODUCTS fetches:
        //   Fetch 1: topProducts { upc } → 2*(Product=1) = 2 (equals max, not individually exceeded)
        //   Fetch 2: _entities for product.name looked up from reviews → adds to aggregate
        //   Aggregate PRODUCTS cost > 2 → blocked by products.max_cost=2
        let res = router
            .send_graphql_request(
                r#"{ topProducts { reviews { product { name } } } }"#,
                None,
                None,
            )
            .await;

        let json = res.json_body().await;

        let blocked_products = json["errors"].as_array().map_or(false, |errors| {
            errors.iter().any(|e| {
                e["extensions"]["code"].as_str() == Some("SUBGRAPH_COST_ESTIMATED_TOO_EXPENSIVE")
                    && e["extensions"]["serviceName"].as_str() == Some("products")
            })
        });
        assert!(
                blocked_products,
                "products should be blocked because aggregate cost across multiple fetches exceeds products.max_cost=2"
            );
    }
    // Per-subgraph list_size override should use a different assumed list size for cost
    // estimation of queries sent to that specific subgraph, independent of the global list_size.
    #[ntex::test]
    async fn named_subgraph_list_size_override_takes_precedence() {
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
                        mode: enforce
                        strategy:
                          static_estimated:
                            list_size: 0
                            max: 1000
                            subgraph:
                                all:
                                    list_size: 3
                                    max: 1000
                                subgraphs:
                                    reviews:
                                        list_size: 10
                                        max: 9
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
        let blocked_reviews = json["errors"].as_array().map_or(false, |errors| {
            errors.iter().any(|e| {
                e["extensions"]["code"].as_str() == Some("SUBGRAPH_COST_ESTIMATED_TOO_EXPENSIVE")
                    && e["extensions"]["serviceName"].as_str() == Some("reviews")
            })
        });

        assert!(
            blocked_reviews,
            "reviews should be blocked because reviews.list_size=10 overrides all.list_size=3"
        );
    }
}
