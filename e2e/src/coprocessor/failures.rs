#[cfg(test)]
mod coprocessor_failures_e2e_tests {
    use sonic_rs::json;

    use crate::testkit::{coprocessor::TestCoprocessor, TestRouter};

    fn default_config(host: &str) -> String {
        format!(
            r#"
            supergraph:
              source: file
              path: supergraph.graphql
            coprocessor:
              url: http://{host}/coprocessor
              protocol: http1
              stages:
                router:
                  request:
                    include:
                      headers: true
            "#
        )
    }

    #[ntex::test]
    async fn rejects_http_error_response() {
        let mut coprocessor = TestCoprocessor::new().await;
        let host = coprocessor.host_with_port();

        let request_stage_mock = coprocessor
            .mock_stage("router.request")
            .with_status(500)
            .with_body("Internal Server Error")
            .expect(1)
            .create();

        let router = TestRouter::builder()
            .inline_config(default_config(&host))
            .build()
            .start()
            .await;

        let response = router
            .send_graphql_request("{ topProducts { name } }", None, None)
            .await;

        assert_eq!(
            response.status().as_u16(),
            500,
            "router should return 500 when coprocessor fails"
        );
        request_stage_mock.assert_async().await;
    }

    #[ntex::test]
    async fn rejects_malformed_json_payload() {
        let mut coprocessor = TestCoprocessor::new().await;
        let host = coprocessor.host_with_port();

        let request_stage_mock = coprocessor
            .mock_stage("router.request")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body("{\"version\": 1, \"control\": ") // Incomplete JSON
            .expect(1)
            .create();

        let router = TestRouter::builder()
            .inline_config(default_config(&host))
            .build()
            .start()
            .await;

        let response = router
            .send_graphql_request("{ topProducts { name } }", None, None)
            .await;

        assert_eq!(
            response.status().as_u16(),
            500,
            "router should return 500 when coprocessor returns malformed JSON"
        );
        request_stage_mock.assert_async().await;
    }

    #[ntex::test]
    async fn rejects_unsupported_version() {
        let mut coprocessor = TestCoprocessor::new().await;
        let host = coprocessor.host_with_port();

        let request_stage_mock = coprocessor
            .mock_stage("router.request")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                json!({
                  "version": 420, // Unsupported version
                  "control": "continue"
                })
                .to_string(),
            )
            .expect(1)
            .create();

        let router = TestRouter::builder()
            .inline_config(default_config(&host))
            .build()
            .start()
            .await;

        let response = router
            .send_graphql_request("{ topProducts { name } }", None, None)
            .await;

        assert_eq!(
            response.status().as_u16(),
            500,
            "router should return 500 when coprocessor returns unsupported version"
        );

        request_stage_mock.assert_async().await;
    }

    #[ntex::test]
    async fn rejects_invalid_control_value() {
        let mut coprocessor = TestCoprocessor::new().await;
        let host = coprocessor.host_with_port();

        let request_stage_mock = coprocessor
            .mock_stage("router.request")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                json!({
                  "version": 1,
                  "control": "jump" // Invalid control value
                })
                .to_string(),
            )
            .expect(1)
            .create();

        let router = TestRouter::builder()
            .inline_config(default_config(&host))
            .build()
            .start()
            .await;

        let response = router
            .send_graphql_request("{ topProducts { name } }", None, None)
            .await;

        assert_eq!(
            response.status().as_u16(),
            500,
            "router should return 500 when coprocessor returns invalid control value"
        );

        request_stage_mock.assert_async().await;
    }
}
