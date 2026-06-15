#[cfg(test)]
mod extensions_tests {
    use super::super::common::*;

    #[ntex::test]
    async fn exposes_cost_headers_when_enabled() {
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
              max: 100
              mode: enforce
              expose_headers:
                estimated: true
                actual: true
                max: true
            subgraphs_budget:
              mode: enforce
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
                    }
                }
                "#,
                None,
                None,
            )
            .await;

        assert_eq!(res.cost_header("x-cost-estimated"), Some(1));
        assert_eq!(res.cost_header("x-cost-actual"), Some(1));
        assert_eq!(res.cost_header("x-cost-max"), Some(100));

        let json = res.json_body().await;
        assert_eq!(json["data"]["me"]["name"].as_str(), Some("Uri Goldshtein"));
        // Cost is exposed via the `X-Cost-*` headers, not response extensions.
        assert!(
            json["extensions"]["cost"].is_null(),
            "cost must not be present in response extensions: {json}"
        );
    }

    #[ntex::test]
    async fn exposes_cost_headers_for_variable_driven_query() {
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
                      expose_headers:
                        estimated: true
                        actual: true
                        max: true
                    subgraphs_budget:
                      mode: enforce
                "#,
            )
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(
                r#"
                query SearchFormulaHasVariable($input: SearchInput!) {
                  search(input: $input) {
                    title
                    author {
                      name
                    }
                  }
                }
                "#,
                Some(json!({
                    "input": {
                        "pagination": { "first": 3 }
                    }
                })),
                None,
            )
            .await;

        // The slicing argument (`first: 3`) drives the estimated cost; the actual
        // cost reflects the three books returned.
        assert_eq!(res.cost_header("x-cost-estimated"), Some(8));
        assert_eq!(res.cost_header("x-cost-actual"), Some(6));
        assert_eq!(res.cost_header("x-cost-max"), Some(1000));

        let json = res.json_body().await;
        assert!(json["errors"].is_null(), "query should succeed: {json}");
    }

    #[ntex::test]
    async fn default_config_exposes_no_cost_headers() {
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
              max: 100
              mode: enforce
            subgraphs_budget:
              mode: enforce
        "#,
            )
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(r#"query { me { name } }"#, None, None)
            .await;

        assert_eq!(res.cost_header("x-cost-estimated"), None);
        assert_eq!(res.cost_header("x-cost-actual"), None);
        assert_eq!(res.cost_header("x-cost-max"), None);

        let json = res.json_body().await;
        assert_eq!(json["data"]["me"]["name"].as_str(), Some("Uri Goldshtein"));
    }

    #[ntex::test]
    async fn cost_header_names_can_be_customized() {
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
              max: 100
              mode: enforce
              expose_headers:
                estimated: "X-My-Estimated"
                actual: "X-My-Actual"
            subgraphs_budget:
              mode: enforce
        "#,
            )
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(r#"query { me { name } }"#, None, None)
            .await;

        assert_eq!(res.cost_header("x-my-estimated"), Some(1));
        assert_eq!(res.cost_header("x-my-actual"), Some(1));

        assert_eq!(res.cost_header("x-cost-estimated"), None);
        assert_eq!(res.cost_header("x-cost-actual"), None);
        // `max` was not enabled at all.
        assert_eq!(res.cost_header("x-cost-max"), None);
    }

    #[ntex::test]
    async fn exposes_only_estimated_header_when_only_estimated_enabled() {
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
              max: 100
              mode: enforce
              expose_headers:
                estimated: true
            subgraphs_budget:
              mode: enforce
        "#,
            )
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(r#"query { me { name } }"#, None, None)
            .await;

        assert_eq!(res.cost_header("x-cost-estimated"), Some(1));
        assert_eq!(res.cost_header("x-cost-actual"), None);
        assert_eq!(res.cost_header("x-cost-max"), None);
    }
}
