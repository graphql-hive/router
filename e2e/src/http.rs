#[cfg(test)]
mod http_tests {
    use std::time::{Duration, Instant};

    use futures::{stream::FuturesUnordered, StreamExt};
    use hive_router::pipeline::execution::EXPOSE_QUERY_PLAN_HEADER;
    use ntex::time;
    use sonic_rs::JsonValueTrait;

    use crate::testkit::{
        some_header_map, wait_until_mock_matched, ClientResponseExt, TestRouter, TestSubgraphs,
    };

    #[ntex::test]
    async fn should_allow_to_customize_graphql_endpoint() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                http:
                    graphql_endpoint: /custom
                "#,
            )
            .build()
            .start()
            .await;

        let json_body = sonic_rs::json!({
          "query": "{ __schema { types { name } } }",
        });

        let res = router
            .serv()
            // even though testrouter will pick up the changed graphql route
            // we use a custom post anyways just to be extra sure
            .post("/custom")
            .content_type("application/json")
            .send_json(&json_body)
            .await
            .expect("failed to send graphql request");

        assert_eq!(res.status(), 200, "Expected 200 OK");

        let res = router
            .serv()
            // we changed and dont use the default
            .post("/graphql")
            .content_type("application/json")
            .send_json(&json_body)
            .await
            .expect("failed to send graphql request");

        assert_eq!(res.status(), 404);
    }

    #[ntex::test]
    async fn should_not_expose_query_plan_when_disabled() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                # default is false
                # query_planner:
                #     allow_expose: false
                "#,
            )
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(
                r#"
                {
                    topProducts {
                        name
                        price
                        reviews {
                            author {
                                name
                            }
                        }
                    }
                }
                "#,
                None,
                some_header_map! {
                    http::header::HeaderName::from_static(EXPOSE_QUERY_PLAN_HEADER.as_str()) => "true"
                },
            )
            .await;

        assert!(res.status().is_success(), "Expected 200 OK");

        let json_body = res.json_body().await;

        assert!(json_body["data"].is_object());
        assert!(json_body["errors"].is_null());
        assert!(json_body["extensions"].is_null());
    }

    #[ntex::test]
    async fn should_execute_and_expose_query_plan() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                query_planner:
                    allow_expose: true
                "#,
            )
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(
                r#"
                {
                    topProducts {
                        name
                        price
                        reviews {
                            author {
                                name
                            }
                        }
                    }
                }
                "#,
                None,
                some_header_map! {
                    http::header::HeaderName::from_static(EXPOSE_QUERY_PLAN_HEADER.as_str()) => "true"
                },
            )
            .await;

        assert!(res.status().is_success(), "Expected 200 OK");

        let json_body = res.json_body().await;

        assert!(json_body["data"].is_object());
        assert!(json_body["errors"].is_null());
        assert!(json_body["extensions"].is_object());
        assert!(json_body["extensions"]["queryPlan"].is_object());
    }

    #[ntex::test]
    async fn should_dry_run_and_expose_query_plan() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                query_planner:
                    allow_expose: true
                "#,
            )
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(
                r#"
                {
                    topProducts {
                        name
                        price
                        reviews {
                            author {
                                name
                            }
                        }
                    }
                }
                "#,
                None,
                some_header_map! {
                    http::header::HeaderName::from_static(EXPOSE_QUERY_PLAN_HEADER.as_str()) => "dry-run"
                },
            )
            .await;

        assert!(res.status().is_success(), "Expected 200 OK");

        let json_body = res.json_body().await;

        assert!(json_body["data"].is_null());
        assert!(json_body["errors"].is_null());
        assert!(json_body["extensions"].is_object());
        assert!(json_body["extensions"]["queryPlan"].is_object());

        assert!(
            subgraphs.get_requests_log("products").is_none(),
            "expected no requests to products subgraph"
        );
    }

    #[ntex::test]
    async fn should_not_dedupe_inflight_router_requests_by_default() {
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
                "#,
            )
            .build()
            .start()
            .await;

        let request_count = 12;
        let mut requests = FuturesUnordered::new();

        for _ in 0..request_count {
            requests.push(router.send_graphql_request(
                r#"
                {
                    topProducts {
                        name
                        price
                    }
                }
                "#,
                None,
                None,
            ));
        }

        while let Some(response) = requests.next().await {
            assert!(response.status().is_success(), "Expected 200 OK");
            let json_body = response.json_body().await;
            assert!(json_body["data"]["topProducts"].is_array());
            assert!(json_body["errors"].is_null());
        }

        let products_requests = subgraphs
            .get_requests_log("products")
            .unwrap_or_default()
            .len();

        assert!(
            products_requests >= request_count,
            "expected at least one products subgraph request per incoming request when router inflight dedupe is disabled by default; got {products_requests} for {request_count} incoming requests"
        );
    }

    #[ntex::test]
    async fn should_dedupe_inflight_router_requests_when_enabled() {
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
                "#,
            )
            .build()
            .start()
            .await;

        let request_count = 12;
        let mut requests = FuturesUnordered::new();

        for _ in 0..request_count {
            requests.push(router.send_graphql_request(
                r#"
                {
                    topProducts {
                        name
                        price
                    }
                }
                "#,
                None,
                None,
            ));
        }

        while let Some(response) = requests.next().await {
            assert!(response.status().is_success(), "Expected 200 OK");
            let json_body = response.json_body().await;
            assert!(json_body["data"]["topProducts"].is_array());
            assert!(json_body["errors"].is_null());
        }

        let products_requests = subgraphs
            .get_requests_log("products")
            .unwrap_or_default()
            .len();

        assert!(
            products_requests < request_count,
            "expected fewer products subgraph requests than incoming requests when router inflight dedupe is enabled; got {products_requests} for {request_count} incoming requests"
        );
    }

    #[ntex::test]
    async fn should_not_dedupe_inflight_router_mutation_requests() {
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
                "#,
            )
            .build()
            .start()
            .await;

        let request_count = 8;
        let mut requests = FuturesUnordered::new();

        for _ in 0..request_count {
            requests.push(router.send_graphql_request(
                r#"
                mutation {
                    oneofTest(input: { string: "router-dedupe" }) {
                        string
                    }
                }
                "#,
                None,
                None,
            ));
        }

        while let Some(response) = requests.next().await {
            assert!(response.status().is_success(), "Expected 200 OK");
        }

        let products_requests = subgraphs
            .get_requests_log("products")
            .unwrap_or_default()
            .len();

        assert!(
            products_requests >= request_count,
            "expected no mutation dedupe at router level; got {products_requests} products requests for {request_count} incoming mutation requests"
        );
    }

    #[ntex::test]
    async fn should_not_share_inflight_dedupe_entry_across_schema_reload() {
        let subgraphs = TestSubgraphs::builder()
            .with_delay(Duration::from_millis(100))
            .build()
            .start()
            .await;

        let initial_supergraph = subgraphs.supergraph(include_str!("../supergraph.graphql"));
        let changed_supergraph = initial_supergraph.replacen("first: Int = 5", "first: Int = 6", 1);
        assert_ne!(initial_supergraph, changed_supergraph);

        let mut server = mockito::Server::new_async().await;
        let host = server.host_with_port();

        let mock_initial = server
            .mock("GET", "/supergraph")
            .expect_at_least(1)
            .with_status(200)
            .with_header("content-type", "text/plain")
            .with_header("etag", "v1")
            .with_body(initial_supergraph)
            .create();

        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(format!(
                r#"
                supergraph:
                    source: hive
                    endpoint: http://{host}/supergraph
                    key: dummy_key
                    poll_interval: 500ms
                traffic_shaping:
                    all:
                        dedupe_enabled: false
                    router:
                        dedupe:
                            enabled: true
                "#,
            ))
            .build()
            .start()
            .await;

        let query = r#"
            {
                topProducts {
                    name
                    price
                }
            }
        "#;

        let first_request = async { router.send_graphql_request(query, None, None).await };
        let second_request = async {
            let started = Instant::now();
            while subgraphs
                .get_requests_log("products")
                .unwrap_or_default()
                .is_empty()
            {
                assert!(
                    started.elapsed() < Duration::from_secs(5),
                    "first request did not reach products subgraph in time"
                );
                time::sleep(Duration::from_millis(100)).await;
            }

            mock_initial.remove();

            let mock_changed = server
                .mock("GET", "/supergraph")
                .expect_at_least(1)
                .with_status(200)
                .with_header("content-type", "text/plain")
                .with_header("etag", "v2")
                .with_body(changed_supergraph)
                .create();

            wait_until_mock_matched(&mock_changed)
                .await
                .expect("expected schema reload poll to fetch changed supergraph");

            router.send_graphql_request(query, None, None).await
        };

        let (first_response, second_response) = futures::join!(first_request, second_request);

        assert!(
            first_response.status().is_success(),
            "expected first request 200 OK"
        );
        assert!(
            second_response.status().is_success(),
            "expected second request 200 OK"
        );

        let products_requests = subgraphs
            .get_requests_log("products")
            .unwrap_or_default()
            .len();

        assert!(
            products_requests >= 2,
            "expected at least two products subgraph requests across schema reload to avoid sharing old in-flight dedupe entry; got {products_requests}"
        );
    }

    #[ntex::test]
    async fn should_use_all_headers_in_router_dedupe_key_by_default() {
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
                "#,
            )
            .build()
            .start()
            .await;

        let query = r#"
            {
                topProducts {
                    name
                    price
                }
            }
        "#;

        let (response_a, response_b) = futures::join!(
            router.send_graphql_request(
                query,
                None,
                some_header_map! {
                    "x-user" => "a"
                },
            ),
            router.send_graphql_request(
                query,
                None,
                some_header_map! {
                    "x-user" => "b"
                },
            )
        );

        assert!(
            response_a.status().is_success(),
            "Expected first request 200 OK"
        );
        assert!(
            response_b.status().is_success(),
            "Expected second request 200 OK"
        );

        let products_requests = subgraphs
            .get_requests_log("products")
            .unwrap_or_default()
            .len();

        assert!(
            products_requests >= 2,
            "expected at least two products subgraph requests when all headers are part of dedupe key; got {products_requests}"
        );
    }

    #[ntex::test]
    async fn should_ignore_headers_in_router_dedupe_key_when_headers_is_empty() {
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
                            headers: none
                "#,
            )
            .build()
            .start()
            .await;

        let query = r#"
            {
                topProducts {
                    name
                    price
                }
            }
        "#;

        let (response_a, response_b) = futures::join!(
            router.send_graphql_request(
                query,
                None,
                some_header_map! {
                    "x-user" => "a"
                },
            ),
            router.send_graphql_request(
                query,
                None,
                some_header_map! {
                    "x-user" => "b"
                },
            )
        );

        assert!(
            response_a.status().is_success(),
            "Expected first request 200 OK"
        );
        assert!(
            response_b.status().is_success(),
            "Expected second request 200 OK"
        );

        let products_requests = subgraphs
            .get_requests_log("products")
            .unwrap_or_default()
            .len();

        assert_eq!(
            products_requests, 1,
            "expected exactly one products subgraph request when router dedupe ignores headers"
        );
    }

    #[ntex::test]
    async fn should_use_case_insensitive_header_allowlist_in_router_dedupe_key() {
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
                            headers:
                                include: ["X-Tenant"]
                "#,
            )
            .build()
            .start()
            .await;

        let query = r#"
            {
                topProducts {
                    name
                    price
                }
            }
        "#;

        let (response_a, response_b) = futures::join!(
            router.send_graphql_request(
                query,
                None,
                some_header_map! {
                    "x-tenant" => "acme",
                    "authorization" => "Bearer token-a"
                },
            ),
            router.send_graphql_request(
                query,
                None,
                some_header_map! {
                    "X-TENANT" => "acme",
                    "authorization" => "Bearer token-b"
                },
            )
        );

        assert!(
            response_a.status().is_success(),
            "Expected first request 200 OK"
        );
        assert!(
            response_b.status().is_success(),
            "Expected second request 200 OK"
        );

        let products_requests = subgraphs
            .get_requests_log("products")
            .unwrap_or_default()
            .len();

        assert_eq!(
            products_requests, 1,
            "expected exactly one products subgraph request when allowlisted header matches case-insensitively"
        );
    }
}
