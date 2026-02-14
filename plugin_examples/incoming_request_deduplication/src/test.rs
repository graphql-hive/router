#[cfg(test)]
mod tests {
    use std::thread::sleep;
    use std::time::Duration;

    use e2e::mockito::{self, ServerOpts};
    use e2e::testkit::{
        init_graphql_request, init_router_from_config_file_with_plugins, wait_for_readiness,
        TestRouterApp,
    };
    use futures::stream::FuturesUnordered;
    use futures::StreamExt;
    use hive_router::http::StatusCode;
    use hive_router::ntex::web::{test, WebResponse};
    use hive_router::sonic_rs::{json, Value};
    use hive_router::{ntex, sonic_rs, PluginRegistry};
    async fn test_parallel_requests<
        T: ntex::Service<ntex::http::Request, Response = WebResponse, Error = ntex::web::Error>,
    >(
        app: &TestRouterApp<T>,
        number_of_parallel_requests: usize,
    ) {
        // There should be 2 requests to accounts subgraph: 1 for the first request, and 1 for the second request that comes in after the first one completes and removes the fingerprint from in-flight requests
        let mut requests = FuturesUnordered::new();
        for _ in 0..number_of_parallel_requests {
            requests.push(test::call_service(
                &app.app,
                init_graphql_request("{ me { name } }", None).to_request(),
            ));
        }

        while let Some(resp) = requests.next().await {
            assert_eq!(resp.status(), StatusCode::OK);
            let json_body: Value = sonic_rs::from_slice(&test::read_body(resp).await).unwrap();
            assert_eq!(
                json_body,
                json!({
                    "data": {
                        "me": {
                            "name": "Uri Goldshtein"
                        }
                    }
                })
            );
        }
    }
    #[ntex::test]
    async fn should_deduplicate_inflight_router_requests() {
        let mut accounts_server = mockito::Server::new_with_opts_async(ServerOpts {
            port: 4200, // Use any available port
            ..Default::default()
        })
        .await;
        let number_of_tests = 5;
        let number_of_parallel_requests = 10;
        let mock = accounts_server
            .mock("POST", "/accounts")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_chunked_body(|w| {
                sleep(Duration::from_millis(300));
                w.write_all(b"{\"data\":{\"me\":{\"name\":\"Uri Goldshtein\"}}}")
            })
            .expect(number_of_tests)
            .create_async()
            .await;
        let app = init_router_from_config_file_with_plugins(
            "../plugin_examples/incoming_request_deduplication/router.config.yaml",
            PluginRegistry::new().register::<crate::plugin::IncomingRequestDeduplicationPlugin>(),
        )
        .await
        .expect("Router should initialize successfully");

        wait_for_readiness(&app.app).await;

        for _ in 0..number_of_tests {
            test_parallel_requests(&app, number_of_parallel_requests).await;
        }

        // Number of parallel requests don't matter
        // As long as the plugin correctly deduplicates in-flight requests,
        // there should be only 1 request to accounts subgraph for each test iteration, regardless of the number of parallel requests

        // Assert that only 1 request was sent to accounts subgraph
        mock.assert_async().await;
    }
}
