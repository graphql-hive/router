#[cfg(test)]
mod extensions_tests {
    use super::super::common::*;
    use crate::testkit::some_header_map;

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

    #[ntex::test]
    async fn exposes_cost_headers_for_deduped_follower_requests() {
        let subgraphs = TestSubgraphs::builder()
            .with_delay(Duration::from_millis(100))
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
        traffic_shaping:
            all:
                dedupe_enabled: false
            router:
                dedupe:
                    enabled: true
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

        let query = r#"query { me { name } }"#;
        let (response_a, response_b) = futures::join!(
            router.send_graphql_request(query, None, None),
            router.send_graphql_request(query, None, None)
        );

        for response in [&response_a, &response_b] {
            assert!(response.status().is_success(), "Expected 200 OK");
            assert_eq!(response.cost_header("x-cost-estimated"), Some(1));
            assert_eq!(response.cost_header("x-cost-actual"), Some(1));
            assert_eq!(response.cost_header("x-cost-max"), Some(100));
        }

        let accounts_requests = subgraphs
            .get_requests_log("accounts")
            .unwrap_or_default()
            .len();
        assert_eq!(
            accounts_requests, 1,
            "expected exactly one accounts subgraph request when router dedupe is enabled"
        );
    }

    #[ntex::test]
    async fn custom_cost_header_names_survive_deduped_follower_requests() {
        let subgraphs = TestSubgraphs::builder()
            .with_delay(Duration::from_millis(100))
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
        traffic_shaping:
            all:
                dedupe_enabled: false
            router:
                dedupe:
                    enabled: true
        demand_control:
            enabled: true
            operation_cost:
              max: 100
              mode: enforce
              expose_headers:
                estimated: "X-My-Estimated"
                actual: "X-My-Actual"
                max: "X-My-Max"
            subgraphs_budget:
              mode: enforce
        "#,
            )
            .build()
            .start()
            .await;

        let query = r#"query { me { name } }"#;
        let (response_a, response_b) = futures::join!(
            router.send_graphql_request(query, None, None),
            router.send_graphql_request(query, None, None)
        );

        for response in [&response_a, &response_b] {
            assert!(response.status().is_success(), "Expected 200 OK");
            assert_eq!(response.cost_header("x-my-estimated"), Some(1));
            assert_eq!(response.cost_header("x-my-actual"), Some(1));
            assert_eq!(response.cost_header("x-my-max"), Some(100));
            assert_eq!(response.cost_header("x-cost-estimated"), None);
            assert_eq!(response.cost_header("x-cost-actual"), None);
            assert_eq!(response.cost_header("x-cost-max"), None);
        }
    }

    #[ntex::test]
    async fn selective_cost_headers_survive_deduped_follower_requests() {
        let subgraphs = TestSubgraphs::builder()
            .with_delay(Duration::from_millis(100))
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
        traffic_shaping:
            all:
                dedupe_enabled: false
            router:
                dedupe:
                    enabled: true
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

        let query = r#"query { me { name } }"#;
        let (response_a, response_b) = futures::join!(
            router.send_graphql_request(query, None, None),
            router.send_graphql_request(query, None, None)
        );

        for response in [&response_a, &response_b] {
            assert!(response.status().is_success(), "Expected 200 OK");
            assert_eq!(response.cost_header("x-cost-estimated"), Some(1));
            assert_eq!(response.cost_header("x-cost-actual"), None);
            assert_eq!(response.cost_header("x-cost-max"), None);
        }
    }

    #[ntex::test]
    async fn does_not_expose_cost_headers_when_estimated_cost_rejects_request() {
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
              max: 0
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
            .send_graphql_request(r#"query { me { name } }"#, None, None)
            .await;
        let json = res.json_body().await;

        assert_eq!(
            json["errors"][0]["extensions"]["code"].as_str(),
            Some("COST_ESTIMATED_TOO_EXPENSIVE")
        );
        assert_eq!(res.cost_header("x-cost-estimated"), Some(1));
        assert_eq!(res.cost_header("x-cost-actual"), None);
        assert_eq!(res.cost_header("x-cost-max"), Some(0));
    }

    #[ntex::test]
    async fn exposes_cost_headers_on_partial_graphql_error_responses() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let mut accounts_server = mockito::Server::new_async().await;
        let host = accounts_server.host_with_port();

        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(format!(
                r#"
        supergraph:
            source: file
            path: supergraph.graphql
        override_subgraph_urls:
            subgraphs:
                accounts:
                    url: "http://{host}/accounts"
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
            ))
            .build()
            .start()
            .await;

        let accounts_response_mock = accounts_server
            .mock("POST", "/accounts")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"data":{"users":[{"id":"1"}]},"errors":[{"message":"upstream is unhappy","extensions":{"code":"UPSTREAM_FAILURE"}}]}"#,
            )
            .expect(1)
            .create();

        let res = router
            .send_graphql_request("{ users { id } }", None, None)
            .await;
        let json = res.json_body().await;

        accounts_response_mock.assert();
        assert!(res.status().is_success(), "Expected 200 OK");
        assert!(json["errors"].is_array(), "expected GraphQL errors: {json}");
        assert_eq!(json["data"]["users"][0]["id"].as_str(), Some("1"));
        assert_eq!(res.cost_header("x-cost-estimated"), Some(0));
        assert_eq!(res.cost_header("x-cost-actual"), Some(1));
        assert_eq!(res.cost_header("x-cost-max"), Some(100));
    }

    #[ntex::test]
    async fn does_not_leak_cost_headers_between_distinct_requests() {
        let subgraphs = TestSubgraphs::builder()
            .with_delay(Duration::from_millis(100))
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
        traffic_shaping:
            all:
                dedupe_enabled: false
            router:
                dedupe:
                    enabled: true
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

        let deduped_query = r#"query { me { name } }"#;
        let (response_a, response_b, response_plain) = futures::join!(
            router.send_graphql_request(deduped_query, None, None),
            router.send_graphql_request(deduped_query, None, None),
            router.send_graphql_request(
                r#"query { topProducts { name price } }"#,
                None,
                some_header_map! { "x-user" => "separate-request" },
            )
        );

        for response in [&response_a, &response_b] {
            assert_eq!(response.cost_header("x-cost-estimated"), Some(1));
        }

        assert!(response_plain.status().is_success(), "Expected 200 OK");
        assert_eq!(response_plain.cost_header("x-cost-estimated"), Some(0));
    }
}
