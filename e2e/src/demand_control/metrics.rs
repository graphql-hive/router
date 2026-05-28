#[cfg(test)]
mod metrics_tests {
    use super::super::common::*;

    // Ensures cost.estimated is emitted with cost.result=COST_ESTIMATED_TOO_EXPENSIVE
    // when global demand control rejects the operation.
    #[ntex::test]
    async fn emits_estimated_metric_for_rejected_operation() {
        let supergraph_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("supergraph_demand_control.graphql");

        let otlp_collector = crate::testkit::otel::OtlpCollector::start()
            .await
            .expect("Failed to start OTLP collector");
        let otlp_endpoint = otlp_collector.http_metrics_endpoint();

        let subgraphs = TestSubgraphs::builder().build().start().await;

        let router = TestRouter::builder()
            .inline_config(format!(
                r#"
                supergraph:
                    source: file
                    path: "{}"

                demand_control:
                    enabled: true
                    mode: enforce
                    strategy:
                      static_estimated:
                        max: 0

                telemetry:
                    metrics:
                        exporters:
                            - kind: otlp
                              endpoint: "{}"
                              protocol: http
                              interval: 30ms
                              max_export_timeout: 50ms
                "#,
                supergraph_path.to_str().unwrap(),
                otlp_endpoint
            ))
            .with_subgraphs(&subgraphs)
            .build()
            .start()
            .await;

        router
            .send_graphql_request(
                r#"
                        query {
                            book(id: 1) {
                                title
                                author { name }
                                publisher { name address { zipCode } }
                            }
                        }"#,
                None,
                None,
            )
            .await;

        wait_for_metrics_export().await;

        let metrics = otlp_collector.metrics_view().await;
        let attrs = [(labels::COST_RESULT, "COST_ESTIMATED_TOO_EXPENSIVE")];

        assert_histogram_sample_count_at_least(&metrics, names::COST_ESTIMATED, &attrs, 1);
    }
    #[ntex::test]
    async fn emits_formula_cache_hit_miss_metrics() {
        let otlp_collector = crate::testkit::otel::OtlpCollector::start()
            .await
            .expect("Failed to start OTLP collector");
        let otlp_endpoint = otlp_collector.http_metrics_endpoint();

        let subgraphs = TestSubgraphs::builder().build().start().await;

        let router = TestRouter::builder()
            .inline_config(format!(
                r#"
                supergraph:
                    source: file
                    path: supergraph_demand_control.graphql

                demand_control:
                    enabled: true
                    mode: enforce
                    strategy:
                      static_estimated:
                        max: 1000

                telemetry:
                    metrics:
                        exporters:
                            - kind: otlp
                              endpoint: "{}"
                              protocol: http
                              interval: 30ms
                              max_export_timeout: 50ms
                "#,
                otlp_endpoint
            ))
            .with_subgraphs(&subgraphs)
            .build()
            .start()
            .await;

        let query = r#"
            query {
              book(id: 1) {
                title
              }
            }
        "#;

        router.send_graphql_request(query, None, None).await;
        router.send_graphql_request(query, None, None).await;

        wait_for_metrics_export().await;
        let metrics = otlp_collector.metrics_view().await;

        assert_counter_at_least(
            &metrics,
            names::DEMAND_CONTROL_FORMULA_CACHE_REQUESTS_TOTAL,
            &[(labels::RESULT, "miss")],
            1.0,
        );
        assert_counter_at_least(
            &metrics,
            names::DEMAND_CONTROL_FORMULA_CACHE_REQUESTS_TOTAL,
            &[(labels::RESULT, "hit")],
            1.0,
        );
    }
    // Ensures cost.estimated, cost.actual and cost.delta are emitted with
    // cost.result=COST_OK for an executed operation.
    #[ntex::test]
    async fn emits_actual_and_delta_metrics_for_executed_operation() {
        let supergraph_path =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("supergraph.graphql");

        let otlp_collector = crate::testkit::otel::OtlpCollector::start()
            .await
            .expect("Failed to start OTLP collector");
        let otlp_endpoint = otlp_collector.http_metrics_endpoint();

        let subgraphs = TestSubgraphs::builder().build().start().await;

        let router = TestRouter::builder()
            .inline_config(format!(
                r#"
                supergraph:
                    source: file
                    path: "{}"

                demand_control:
                    enabled: true
                    mode: enforce
                    strategy:
                      static_estimated:
                        max: 1000
                        actual_cost_mode: by_response_shape
                    include_extension_metadata: true
                telemetry:
                    metrics:
                        exporters:
                            - kind: otlp
                              endpoint: "{}"
                              protocol: http
                              interval: 30ms
                              max_export_timeout: 50ms
                "#,
                supergraph_path.to_str().unwrap(),
                otlp_endpoint
            ))
            .with_subgraphs(&subgraphs)
            .build()
            .start()
            .await;

        router
            .send_graphql_request(
                r#"
                        query {
                            me {
                                reviews {
                                    body
                                }
                            }
                        }"#,
                None,
                None,
            )
            .await;

        wait_for_metrics_export().await;

        let metrics = otlp_collector.metrics_view().await;
        let attrs = [(labels::COST_RESULT, "COST_OK")];

        assert_histogram_sample_count_at_least(&metrics, names::COST_ESTIMATED, &attrs, 1);
        assert_histogram_sample_count_at_least(&metrics, names::COST_ACTUAL, &attrs, 1);
        assert_histogram_sample_count_at_least(&metrics, names::COST_DELTA, &attrs, 1);
    }
    // Ensures cost.actual and cost.delta are emitted with
    // cost.result=COST_ACTUAL_TOO_EXPENSIVE when execution exceeds max_cost.
    #[ntex::test]
    async fn emits_actual_and_delta_metrics_for_actual_too_expensive_operation() {
        let supergraph_path =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("supergraph.graphql");

        let otlp_collector = crate::testkit::otel::OtlpCollector::start()
            .await
            .expect("Failed to start OTLP collector");
        let otlp_endpoint = otlp_collector.http_metrics_endpoint();

        let subgraphs = TestSubgraphs::builder().build().start().await;

        let router = TestRouter::builder()
            .inline_config(format!(
                r#"
                supergraph:
                    source: file
                    path: "{}"

                demand_control:
                    enabled: true
                    mode: enforce
                    strategy:
                      static_estimated:
                        list_size: 0
                        max: 3
                        actual_cost_mode: by_subgraph
                    include_extension_metadata: true
                telemetry:
                    metrics:
                        exporters:
                            - kind: otlp
                              endpoint: "{}"
                              protocol: http
                              interval: 30ms
                              max_export_timeout: 50ms
                "#,
                supergraph_path.to_str().unwrap(),
                otlp_endpoint
            ))
            .with_subgraphs(&subgraphs)
            .build()
            .start()
            .await;

        router
            .send_graphql_request(
                r#"
                        query {
                            me {
                                reviews {
                                    body
                                }
                            }
                        }"#,
                None,
                None,
            )
            .await;

        wait_for_metrics_export().await;

        let metrics = otlp_collector.metrics_view().await;
        let estimated_ok_attrs = [(labels::COST_RESULT, "COST_OK")];
        let actual_too_expensive_attrs = [(labels::COST_RESULT, "COST_ACTUAL_TOO_EXPENSIVE")];

        assert_histogram_sample_count(&metrics, names::COST_ESTIMATED, &estimated_ok_attrs, 1);
        assert_histogram_sample_count(&metrics, names::COST_ACTUAL, &actual_too_expensive_attrs, 1);
        assert_histogram_sample_count(&metrics, names::COST_DELTA, &actual_too_expensive_attrs, 1);
    }
    // Ensures standard demand-control histograms carry graphql.operation.name alongside cost.result.
    #[ntex::test]
    async fn emits_demand_control_metrics_with_operation_name_attribute() {
        let supergraph_path =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("supergraph.graphql");

        let otlp_collector = OtlpCollector::start()
            .await
            .expect("Failed to start OTLP collector");
        let otlp_endpoint = otlp_collector.http_metrics_endpoint();

        let subgraphs = TestSubgraphs::builder().build().start().await;

        let router = TestRouter::builder()
            .inline_config(format!(
                r#"
                supergraph:
                    source: file
                    path: "{}"

                demand_control:
                    enabled: true
                    mode: enforce
                    strategy:
                      static_estimated:
                        list_size: 10
                        max: 1000
                        actual_cost_mode: by_response_shape
                    include_extension_metadata: true
                telemetry:
                    metrics:
                        exporters:
                            - kind: otlp
                              endpoint: "{}"
                              protocol: http
                              interval: 30ms
                              max_export_timeout: 50ms
                "#,
                supergraph_path.to_str().unwrap(),
                otlp_endpoint
            ))
            .with_subgraphs(&subgraphs)
            .build()
            .start()
            .await;

        router
            .send_graphql_request(
                r#"
                query DemandControlNamedQuery {
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

        wait_for_metrics_export().await;

        let metrics = otlp_collector.metrics_view().await;
        let attrs = [
            (labels::COST_RESULT, "COST_OK"),
            (labels::GRAPHQL_OPERATION_NAME, "DemandControlNamedQuery"),
        ];

        assert_histogram_sample_count(&metrics, names::COST_ESTIMATED, &attrs, 1);
        assert_histogram_sample_count(&metrics, names::COST_ACTUAL, &attrs, 1);
        assert_histogram_sample_count(&metrics, names::COST_DELTA, &attrs, 1);
    }
    // Ensures cost.estimated/cost.actual/cost.delta/cost.result are attached to graphql.operation spans.
    #[ntex::test]
    async fn records_demand_control_cost_attributes_on_graphql_operation_span() {
        let supergraph_path =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("supergraph.graphql");

        let otlp_collector = OtlpCollector::start()
            .await
            .expect("Failed to start OTLP collector");
        let otlp_endpoint = otlp_collector.http_traces_endpoint();

        let subgraphs = TestSubgraphs::builder().build().start().await;

        let router = TestRouter::builder()
            .inline_config(format!(
                r#"
                supergraph:
                    source: file
                    path: {}

                demand_control:
                    enabled: true
                    mode: enforce
                    strategy:
                      static_estimated:
                        list_size: 10
                        max: 1000
                        actual_cost_mode: by_response_shape
                    include_extension_metadata: true
                telemetry:
                    tracing:
                      exporters:
                        - kind: otlp
                          endpoint: {otlp_endpoint}
                          protocol: http
                          batch_processor:
                            scheduled_delay: 50ms
                            max_export_timeout: 50ms
                "#,
                supergraph_path.to_str().unwrap(),
            ))
            .with_subgraphs(&subgraphs)
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(
                r#"
                query DemandControlSpanQuery {
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

        assert!(res.status().is_success());

        let operation_span = otlp_collector
            .wait_for_span_by_hive_kind_one("graphql.operation")
            .await;

        assert_eq!(
            operation_span.attributes.get("graphql.operation.name"),
            Some(&"DemandControlSpanQuery".to_string())
        );
        assert_eq!(
            operation_span.attributes.get("cost.result"),
            Some(&"COST_OK".to_string())
        );
        assert_eq!(
            operation_span.attributes.get("cost.formula_cache_hit"),
            Some(&"false".to_string())
        );

        let estimated = operation_span
            .attributes
            .get("cost.estimated")
            .expect("operation span should include cost.estimated")
            .parse::<u64>()
            .expect("cost.estimated should be a u64");
        let actual = operation_span
            .attributes
            .get("cost.actual")
            .expect("operation span should include cost.actual")
            .parse::<u64>()
            .expect("cost.actual should be a u64");
        let delta = operation_span
            .attributes
            .get("cost.delta")
            .expect("operation span should include cost.delta")
            .parse::<i64>()
            .expect("cost.delta should be an i64");

        assert!(actual < estimated);
        assert_eq!(delta, actual as i64 - estimated as i64);
    }
    #[ntex::test]
    async fn does_not_record_demand_control_attributes_when_demand_control_is_disabled() {
        let supergraph_path =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("supergraph.graphql");

        let otlp_collector = OtlpCollector::start()
            .await
            .expect("Failed to start OTLP collector");
        let otlp_endpoint = otlp_collector.http_traces_endpoint();

        let subgraphs = TestSubgraphs::builder().build().start().await;

        let router = TestRouter::builder()
            .inline_config(format!(
                r#"
                supergraph:
                    source: file
                    path: {}

                demand_control:
                    enabled: false
                    mode: enforce
                    strategy:
                        static_estimated:
                            max: 100

                telemetry:
                    tracing:
                      exporters:
                        - kind: otlp
                          endpoint: {otlp_endpoint}
                          protocol: http
                          batch_processor:
                            scheduled_delay: 50ms
                            max_export_timeout: 50ms
                "#,
                supergraph_path.to_str().unwrap(),
            ))
            .with_subgraphs(&subgraphs)
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(
                r#"
                query {
                  me { id }
                }
                "#,
                None,
                None,
            )
            .await;

        assert!(res.status().is_success());

        let operation_span = otlp_collector
            .wait_for_span_by_hive_kind_one("graphql.operation")
            .await;

        assert!(
            operation_span.attributes.get("cost.estimated").is_none(),
            "operation span should not include cost.estimated when demand control is disabled"
        );
        assert!(
            operation_span.attributes.get("cost.actual").is_none(),
            "operation span should not include cost.actual when demand control is disabled"
        );
        assert!(
            operation_span.attributes.get("cost.delta").is_none(),
            "operation span should not include cost.delta when demand control is disabled"
        );
        assert!(
            operation_span.attributes.get("cost.result").is_none(),
            "operation span should not include cost.result when demand control is disabled"
        );
        assert!(
            operation_span
                .attributes
                .get("cost.formula_cache_hit")
                .is_none(),
            "operation span should not include cost.formula_cache_hit when demand control is disabled"
        );
    }
}
