#[cfg(test)]
mod tests {
    use std::thread::sleep;
    use std::time::Duration;

    use e2e::mockito;
    use e2e::testkit::{ClientResponseExt, EnvVarsGuard, TestRouterBuilder};
    use futures::stream::FuturesUnordered;
    use futures::StreamExt;
    use hive_router::{http::StatusCode, ntex, sonic_rs};

    #[ntex::test]
    async fn should_deduplicate_inflight_router_requests() {
        let mut accounts_server = mockito::Server::new_async().await;

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

        let _env_guard = EnvVarsGuard::new()
            .set(
                "ACCOUNTS_URL_OVERRIDE",
                &format!("http://{}/accounts", accounts_server.host_with_port()),
            )
            .apply()
            .await;

        let router = TestRouterBuilder::new()
            .file_config("../plugin_examples/incoming_request_deduplication/router.config.yaml")
            .register_plugin::<crate::plugin::IncomingRequestDeduplicationPlugin>()
            .build()
            .start()
            .await;

        for _ in 0..number_of_tests {
            let mut requests = FuturesUnordered::new();
            for _ in 0..number_of_parallel_requests {
                requests.push(router.send_graphql_request("{ me { name } }", None, None));
            }

            while let Some(res) = requests.next().await {
                assert_eq!(res.status(), StatusCode::OK);
                assert_eq!(
                    res.json_body().await,
                    sonic_rs::json!({
                        "data": {
                            "me": {
                                "name": "Uri Goldshtein"
                            }
                        }
                    })
                );
            }
        }

        // Number of parallel requests don't matter
        // As long as the plugin correctly deduplicates in-flight requests,
        // there should be only 1 request to accounts subgraph for each test iteration, regardless of the number of parallel requests

        // Assert that only 1 request was sent to accounts subgraph per iteration
        mock.assert_async().await;
    }
}
