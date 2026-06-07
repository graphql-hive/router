#[cfg(test)]
mod enforcement_tests {
    use super::super::common::*;

    #[ntex::test]
    async fn rejects_request_when_estimated_cost_exceeds_max() {
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
                max: 0
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

        let json = res.json_body().await;
        assert_eq!(
            json["errors"][0]["message"].as_str(),
            Some("Operation estimated cost 1 exceeds configured max cost 0")
        );
        assert_eq!(
            json["errors"][0]["extensions"]["code"].as_str(),
            Some("COST_ESTIMATED_TOO_EXPENSIVE")
        );
    }
    // Mode: measure (dry-run) - cost calculated but operation NOT rejected even if it exceeds max_cost.
    #[ntex::test]
    async fn mode_measure_always_allows_operation() {
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
                    mode: measure
                    strategy:
                      static_estimated:
                        max: 0
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
              }
            }
            "#,
                None,
                None,
            )
            .await;

        let json = res.json_body().await;
        assert!(json.get("errors").is_none() || json["errors"].is_null());
        assert_eq!(json["data"]["me"]["name"].as_str(), Some("Uri Goldshtein"));
        assert_eq!(res.cost_header("x-cost-estimated"), Some(1));
        assert_eq!(res.cost_header("x-cost-actual"), Some(1));
        assert_eq!(res.cost_header("x-cost-max"), Some(0));
    }
    #[ntex::test]
    async fn mode_measure_does_not_reject_when_actual_cost_exceeds_max() {
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
                    mode: measure
                    strategy:
                      static_estimated:
                        list_size: 0
                        max: 3
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
            "measure mode must not reject the operation when actual cost exceeds max"
        );
        assert_eq!(res.cost_header("x-cost-max"), Some(3));
    }
    #[ntex::test]
    async fn subscription_is_rejected_when_estimated_cost_exceeds_max() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                    supergraph:
                        source: file
                        path: supergraph.graphql
                    websocket:
                        enabled: true
                    subscriptions:
                        enabled: true
                    demand_control:
                        enabled: true
                        mode: enforce
                        strategy:
                          static_estimated:
                            max: 0
                "#,
            )
            .build()
            .start()
            .await;

        let wsconn = router.ws().await;
        let mut client = WsClient::init(wsconn, None)
            .await
            .expect("Failed to init WsClient");

        let subscribe_payload = SubscribePayload {
            query: r#"
                subscription {
                    reviewAdded(step: 1, intervalInMs: 0) {
                        id
                        body
                    }
                }
                "#
            .into(),
            ..Default::default()
        };

        let mut stream = client.subscribe(subscribe_payload, None).await;
        let first = stream.next().await.expect("Expected a rejection response");
        let errors = first
            .errors
            .expect("Expected errors for over-budget subscription");

        assert_eq!(
            errors[0].extensions.code.as_deref(),
            Some("COST_ESTIMATED_TOO_EXPENSIVE")
        );
        assert!(
            errors[0].message.contains("Operation estimated cost"),
            "unexpected demand-control error message: {}",
            errors[0].message
        );

        let next = stream.next().await;
        assert!(
            next.is_none(),
            "Expected subscription stream to complete after demand-control rejection"
        );
    }
}
