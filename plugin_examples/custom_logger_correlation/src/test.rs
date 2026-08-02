#[cfg(test)]
mod apollo_sandbox_tests {
    use e2e::testkit::{TestRouter, TestSubgraphs};
    use hive_router::{ntex, sonic_rs};
    use e2e::testkit::stdout::CaptureStdoutExt;

    #[ntex::test]
    async fn correlates_log_lines_correctly() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .file_config("../plugin_examples/custom_logger_correlation/router.config.yaml")
            .register_plugin::<crate::plugin::CustomLoggerCorrelationPlugin>()
            .with_subgraphs(&subgraphs)
            .build()
            .start()
            .await;

        let stdout_log = router
            .send_post_request(
                "/test/graphql",
                sonic_rs::json!({
                  "query": "{ __typename }",
                }),
                None,
            )
            .capture_stdout_json()
            .await;

        let http_req_start_line = stdout_log.by_message("http request started").expect("missing http request started line");
        let custom_correlation = http_req_start_line.get("project_id").and_then(serde_json::Value::as_str).map(|s| s.to_string());
        assert_eq!(custom_correlation, Some("test".to_string()));

        // TODO: Once we fix correlation on custom plugins, this can be enabled.
        //let custom_correlation = http_req_start_line.get("project_id").and_then(serde_json::Value::as_str).map(|s| s.to_string());
        // let custom_line = stdout_log.by_message("on_http_request called").expect("missing custom line");
        // let custom_correlation = custom_line.get("project_id").and_then(serde_json::Value::as_str).map(|s| s.to_string());
        // assert_eq!(custom_correlation, Some("test".to_string()));
    }
}
