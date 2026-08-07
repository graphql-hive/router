#[cfg(test)]
mod custom_logger_correlation_tests {
    use e2e::testkit::stdout::CaptureStdoutExt;
    use e2e::testkit::{TestRouter, TestSubgraphs};
    use hive_router::{ntex, sonic_rs};

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

        // The plugin's own log line, emitted right after it sets the correlation, already
        // carries it - unlike the callback-based approach this replaces, which could only
        // correlate log lines emitted after the whole request-identifiers extraction step.
        let plugin_log_line = stdout_log
            .by_message("on_http_request called")
            .expect("missing on_http_request log line");
        assert_eq!(
            plugin_log_line
                .get("project_id")
                .and_then(serde_json::Value::as_str),
            Some("test")
        );

        // Every log line emitted afterwards for the same request carries it too.
        let http_req_start_line = stdout_log
            .by_message("http request started")
            .expect("missing http request started line");
        assert_eq!(
            http_req_start_line
                .get("project_id")
                .and_then(serde_json::Value::as_str),
            Some("test")
        );

        // The request summary line's message was customized by the plugin, using data
        // (status code, duration, operation name) only known once the request is done.
        let summary_line = stdout_log
            .lines_json
            .iter()
            .find(|line| {
                line.get("target").and_then(serde_json::Value::as_str) == Some("router::request")
            })
            .expect("missing request summary line");
        let message = summary_line
            .get("message")
            .and_then(serde_json::Value::as_str)
            .expect("missing summary message");
        assert!(
            message.starts_with("[status=200] ["),
            "unexpected summary message: {message}"
        );
        // the query is anonymous, so the operation name falls back to "-"
        assert!(
            message.ends_with("] POST /test/graphql -"),
            "unexpected summary message: {message}"
        );
    }
}
