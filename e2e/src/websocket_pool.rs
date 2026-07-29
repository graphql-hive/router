#[cfg(test)]
mod websocket_pool_e2e_tests {
    use std::time::Duration;

    use futures::StreamExt;
    use sonic_rs::JsonValueTrait;

    use crate::testkit::{some_header_map, ClientResponseExt, TestRouter, TestSubgraphs};

    const LOOPING_SUBSCRIPTION: &str = r#"
        subscription {
            reviewAddedLooping(intervalInMs: 20) {
                id
            }
        }
    "#;

    const REVIEWS_QUERY: &str = r#"
        query {
            topProducts(first: 1) {
                reviews {
                    id
                }
            }
        }
    "#;

    fn sse_headers() -> Option<http::HeaderMap> {
        some_header_map! {
            http::header::ACCEPT => "text/event-stream"
        }
    }

    #[ntex::test]
    async fn concurrent_subscriptions_share_one_websocket_connection() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .file_config("configs/websocket_pool.yaml")
            .build()
            .start()
            .await;

        let first = router.send_graphql_request(
            "subscription { reviewAdded(step: 2, intervalInMs: 0) { id } }",
            None,
            sse_headers(),
        );
        let second = router.send_graphql_request(
            "subscription { reviewAdded(step: 3, intervalInMs: 0) { id } }",
            None,
            sse_headers(),
        );
        let (first, second) = tokio::join!(first, second);
        let (first, second) = tokio::join!(first.string_body(), second.string_body());

        assert!(first.contains(r#""id":"1""#) && first.contains("event: complete"));
        assert!(second.contains(r#""id":"1""#) && second.contains("event: complete"));
        assert_eq!(
            subgraphs
                .get_requests_log("reviews/ws")
                .unwrap_or_default()
                .len(),
            1,
            "concurrent initialization should perform one websocket upgrade"
        );
        assert!(
            subgraphs.get_requests_log("reviews").is_none(),
            "websocket subscriptions should not use the reviews HTTP endpoint"
        );
    }

    #[ntex::test]
    async fn different_operations_multiplex_on_the_initialized_connection() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .file_config("configs/websocket_pool.yaml")
            .build()
            .start()
            .await;

        let mut subscription = router
            .send_graphql_request(LOOPING_SUBSCRIPTION, None, sse_headers())
            .await;
        subscription.next().await.unwrap().unwrap();

        let first = router.send_graphql_request(REVIEWS_QUERY, None, None);
        let second = router.send_graphql_request(
            "query { topProducts(first: 2) { reviews { body } } }",
            None,
            None,
        );
        let (first, second) = tokio::join!(first, second);

        let first = first.json_body().await;
        let second = second.json_body().await;
        assert!(first.get("data").is_some() && first.get("errors").is_none());
        assert!(second.get("data").is_some() && second.get("errors").is_none());
        assert_eq!(
            subgraphs
                .get_requests_log("reviews/ws")
                .unwrap_or_default()
                .len(),
            1
        );
        assert!(
            subgraphs.get_requests_log("reviews").is_none(),
            "both review entity fetches should use the pooled websocket"
        );
    }

    #[ntex::test]
    async fn query_uses_http_when_no_initialized_connection_exists() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .file_config("configs/websocket_pool.yaml")
            .build()
            .start()
            .await;

        let response = router
            .send_graphql_request(REVIEWS_QUERY, None, None)
            .await
            .json_body()
            .await;

        assert!(response.get("data").is_some());
        assert_eq!(
            subgraphs
                .get_requests_log("reviews")
                .unwrap_or_default()
                .len(),
            1
        );
        assert!(
            subgraphs.get_requests_log("reviews/ws").is_none(),
            "a query must not initialize a websocket"
        );
    }

    #[ntex::test]
    async fn query_uses_http_while_the_matching_connection_is_initializing() {
        let subgraphs = TestSubgraphs::builder()
            .with_path_delay("/reviews/ws", Duration::from_millis(300))
            .build()
            .start()
            .await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .file_config("configs/websocket_pool.yaml")
            .build()
            .start()
            .await;

        let subscription = router.send_graphql_request(
            "subscription { reviewAdded(step: 11, intervalInMs: 0) { id } }",
            None,
            sse_headers(),
        );
        let query = async {
            for _ in 0..100 {
                if subgraphs.get_requests_log("reviews/ws").is_some() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            assert!(
                subgraphs.get_requests_log("reviews/ws").is_some(),
                "subscription did not begin websocket initialization"
            );
            router.send_graphql_request(REVIEWS_QUERY, None, None).await
        };
        let (subscription, query) = tokio::join!(subscription, query);

        assert!(subscription.string_body().await.contains("event: complete"));
        assert!(query.json_body().await.get("data").is_some());
        assert_eq!(
            subgraphs
                .get_requests_log("reviews")
                .unwrap_or_default()
                .len(),
            1,
            "a query must not wait for a connecting websocket"
        );
    }

    #[ntex::test]
    async fn http_mode_never_creates_websocket_connections_for_queries_or_mutations() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                subscriptions:
                    enabled: true
                    websocket:
                        subgraphs:
                            products:
                                path: /reviews/ws
                traffic_shaping:
                    router:
                        dedupe:
                            headers: none
                "#,
            )
            .build()
            .start()
            .await;

        let query = router
            .send_graphql_request("query { topProducts(first: 1) { name } }", None, None)
            .await
            .json_body()
            .await;
        let mutation = router
            .send_graphql_request("mutation { reentryTest { ok } }", None, None)
            .await
            .json_body()
            .await;

        assert!(query.get("data").is_some());
        assert!(mutation.get("data").is_some());
        assert_eq!(
            subgraphs
                .get_requests_log("products")
                .unwrap_or_default()
                .len(),
            2
        );
        assert!(
            subgraphs.get_requests_log("reviews/ws").is_none(),
            "execute-only traffic must not perform a websocket upgrade"
        );
    }

    #[ntex::test]
    async fn http_mode_does_not_reuse_an_initialized_connection() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                subscriptions:
                    enabled: true
                    websocket:
                        subgraphs:
                            reviews:
                                path: /reviews/ws
                traffic_shaping:
                    all:
                        websocket:
                            execute_mode: http
                    router:
                        dedupe:
                            headers: none
                "#,
            )
            .build()
            .start()
            .await;

        let mut subscription = router
            .send_graphql_request(LOOPING_SUBSCRIPTION, None, sse_headers())
            .await;
        subscription.next().await.unwrap().unwrap();

        let response = router
            .send_graphql_request(REVIEWS_QUERY, None, None)
            .await
            .json_body()
            .await;

        assert!(response.get("data").is_some() && response.get("errors").is_none());
        assert_eq!(
            subgraphs
                .get_requests_log("reviews")
                .unwrap_or_default()
                .len(),
            1,
            "http mode must ignore an eligible pooled websocket"
        );
        assert_eq!(
            subgraphs
                .get_requests_log("reviews/ws")
                .unwrap_or_default()
                .len(),
            1
        );
    }

    #[ntex::test]
    async fn websocket_mode_initializes_and_reuses_a_connection_for_queries() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                subscriptions:
                    enabled: true
                    websocket:
                        subgraphs:
                            reviews:
                                path: /reviews/ws
                traffic_shaping:
                    all:
                        pool_idle_timeout: 5s
                        websocket:
                            execute_mode: websocket
                    subgraphs:
                        products:
                            websocket:
                                execute_mode: http
                    router:
                        dedupe:
                            headers: none
                "#,
            )
            .build()
            .start()
            .await;

        let first = router
            .send_graphql_request(REVIEWS_QUERY, None, None)
            .await
            .json_body()
            .await;
        let second = router
            .send_graphql_request(REVIEWS_QUERY, None, None)
            .await
            .json_body()
            .await;

        assert!(first.get("data").is_some() && first.get("errors").is_none());
        assert!(second.get("data").is_some() && second.get("errors").is_none());
        assert_eq!(
            subgraphs
                .get_requests_log("products")
                .unwrap_or_default()
                .len(),
            2,
            "the products subgraph override should keep its fetches on HTTP"
        );
        assert_eq!(
            subgraphs
                .get_requests_log("reviews/ws")
                .unwrap_or_default()
                .len(),
            1,
            "websocket mode should initialize once and reuse the pooled connection"
        );
        assert!(
            subgraphs.get_requests_log("reviews").is_none(),
            "websocket mode should not execute the review fetch over HTTP"
        );
    }

    #[ntex::test]
    async fn websocket_mode_executes_every_subgraph_fetch_over_websocket() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                subscriptions:
                    websocket:
                        subgraphs:
                            reviews:
                                path: /reviews/ws
                            accounts:
                                path: /accounts/ws
                            inventory:
                                path: /inventory/ws
                            products:
                                path: /products/ws
                traffic_shaping:
                    all:
                        websocket:
                            execute_mode: websocket
                    router:
                        dedupe:
                            headers: none
                "#,
            )
            .build()
            .start()
            .await;

        // run 3 queries making sure we're using the pool
        for _ in 0..3 {
            let response = router
                .send_graphql_request(
                    r#"
                    fragment User on User {
                        id
                        username
                        name
                    }

                    fragment Review on Review {
                        id
                        body
                    }

                    fragment Product on Product {
                        inStock
                        name
                        price
                        shippingEstimate
                        upc
                        weight
                    }

                    query TestQuery {
                        users {
                            ...User
                            reviews {
                                ...Review
                                product {
                                    ...Product
                                    reviews {
                                        ...Review
                                        author {
                                            ...User
                                            reviews {
                                                ...Review
                                                product {
                                                    ...Product
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        topProducts(first: 1) {
                            ...Product
                            reviews {
                                ...Review
                                author {
                                    ...User
                                    reviews {
                                        ...Review
                                        product {
                                            ...Product
                                        }
                                    }
                                }
                            }
                        }
                    }
                    "#,
                    None,
                    None,
                )
                .await
                .json_body()
                .await;

            // response is too big for snapshot assertion, so just do a bunch of sanity checks
            assert!(response.get("errors").is_none(), "{response}");
            assert_eq!(
                response["data"]["users"][0]["name"].as_str(),
                Some("Uri Goldshtein")
            );
            assert_eq!(
                response["data"]["topProducts"][0]["name"].as_str(),
                Some("Table")
            );
            assert_eq!(
                response["data"]["topProducts"][0]["inStock"].as_bool(),
                Some(true)
            );
            assert_eq!(
                response["data"]["topProducts"][0]["reviews"][0]["id"].as_str(),
                Some("1")
            );
        }

        for subgraph in ["accounts", "inventory", "products", "reviews"] {
            assert_eq!(
                subgraphs
                    .get_requests_log(&format!("{subgraph}/ws"))
                    .unwrap_or_default()
                    .len(),
                1,
                "{subgraph} should use one websocket"
            );
            assert!(subgraphs.get_requests_log(subgraph).is_none());
        }
    }

    #[ntex::test]
    async fn websocket_mode_without_reuse_opens_one_connection_per_query() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                subscriptions:
                    enabled: true
                    websocket:
                        subgraphs:
                            reviews:
                                path: /reviews/ws
                traffic_shaping:
                    all:
                        websocket:
                            reuse_connections: false
                            execute_mode: websocket
                    subgraphs:
                        products:
                            websocket:
                                execute_mode: http
                    router:
                        dedupe:
                            headers: none
                "#,
            )
            .build()
            .start()
            .await;

        let first = router
            .send_graphql_request(REVIEWS_QUERY, None, None)
            .await
            .json_body()
            .await;
        let second = router
            .send_graphql_request(REVIEWS_QUERY, None, None)
            .await
            .json_body()
            .await;

        assert!(first.get("data").is_some() && first.get("errors").is_none());
        assert!(second.get("data").is_some() && second.get("errors").is_none());
        assert_eq!(
            subgraphs
                .get_requests_log("reviews/ws")
                .unwrap_or_default()
                .len(),
            2,
            "disabled reuse should open a dedicated websocket for each query"
        );
        assert!(subgraphs.get_requests_log("reviews").is_none());
    }

    #[ntex::test]
    async fn disabled_connection_reuse_gives_each_subscription_its_own_connection() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                subscriptions:
                    enabled: true
                    websocket:
                        subgraphs:
                            reviews:
                                path: /reviews/ws
                traffic_shaping:
                    all:
                        pool_idle_timeout: 5s
                        websocket:
                            reuse_connections: false
                            execute_mode: http
                    router:
                        dedupe:
                            headers: none
                "#,
            )
            .build()
            .start()
            .await;

        let first = router.send_graphql_request(
            "subscription { reviewAdded(step: 2, intervalInMs: 0) { id } }",
            None,
            sse_headers(),
        );
        let second = router.send_graphql_request(
            "subscription { reviewAdded(step: 3, intervalInMs: 0) { id } }",
            None,
            sse_headers(),
        );
        let (first, second) = tokio::join!(first, second);
        let (first, second) = tokio::join!(first.string_body(), second.string_body());

        assert!(first.contains("event: complete"));
        assert!(second.contains("event: complete"));
        assert_eq!(
            subgraphs
                .get_requests_log("reviews/ws")
                .unwrap_or_default()
                .len(),
            2,
            "disabling reuse should preserve one WebSocket per subscription"
        );
    }

    #[ntex::test]
    async fn dropping_one_operation_leaves_the_connection_reusable() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .file_config("configs/websocket_pool.yaml")
            .build()
            .start()
            .await;

        let mut subscription = router
            .send_graphql_request(LOOPING_SUBSCRIPTION, None, sse_headers())
            .await;
        subscription.next().await.unwrap().unwrap();
        drop(subscription);

        for _ in 0..100 {
            if subgraphs.active_subscriptions() == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert_eq!(subgraphs.active_subscriptions(), 0);

        let response = router
            .send_graphql_request(REVIEWS_QUERY, None, None)
            .await
            .json_body()
            .await;

        assert!(response.get("data").is_some() && response.get("errors").is_none());
        assert_eq!(
            subgraphs
                .get_requests_log("reviews/ws")
                .unwrap_or_default()
                .len(),
            1,
            "cancelling a logical operation must not close the pooled connection"
        );
        assert!(subgraphs.get_requests_log("reviews").is_none());
    }

    #[ntex::test]
    async fn active_operation_prevents_idle_expiry() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                subscriptions:
                    enabled: true
                    websocket:
                        subgraphs:
                            reviews:
                                path: /reviews/ws
                traffic_shaping:
                    all:
                        pool_idle_timeout: 50ms
                        websocket:
                            execute_mode: reuse_existing
                    router:
                        dedupe:
                            headers: none
                "#,
            )
            .build()
            .start()
            .await;

        let mut subscription = router
            .send_graphql_request(LOOPING_SUBSCRIPTION, None, sse_headers())
            .await;
        subscription.next().await.unwrap().unwrap();
        tokio::time::sleep(Duration::from_millis(150)).await;

        let response = router
            .send_graphql_request(REVIEWS_QUERY, None, None)
            .await
            .json_body()
            .await;

        assert!(response.get("data").is_some() && response.get("errors").is_none());
        assert_eq!(
            subgraphs
                .get_requests_log("reviews/ws")
                .unwrap_or_default()
                .len(),
            1
        );
        assert!(
            subgraphs.get_requests_log("reviews").is_none(),
            "an active operation must keep the connection out of the idle state"
        );
    }

    // actually testing pool_idle_timeout
    #[ntex::test]
    async fn websocket_mode_reconnects_after_pool_idle_timeout() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                subscriptions:
                    enabled: true
                    websocket:
                        subgraphs:
                            reviews:
                                path: /reviews/ws
                traffic_shaping:
                    all:
                        pool_idle_timeout: 50ms
                        websocket:
                            execute_mode: websocket
                    router:
                        dedupe:
                            headers: none
                "#,
            )
            .build()
            .start()
            .await;

        let first = router
            .send_graphql_request(REVIEWS_QUERY, None, None)
            .await
            .json_body()
            .await;
        let second = router
            .send_graphql_request(REVIEWS_QUERY, None, None)
            .await
            .json_body()
            .await;

        assert!(first.get("data").is_some() && first.get("errors").is_none());
        assert!(second.get("data").is_some() && second.get("errors").is_none());
        assert_eq!(
            subgraphs
                .get_requests_log("reviews/ws")
                .unwrap_or_default()
                .len(),
            1,
            "queries before the idle timeout should reuse one websocket"
        );

        tokio::time::sleep(Duration::from_millis(150)).await;

        let third = router
            .send_graphql_request(REVIEWS_QUERY, None, None)
            .await
            .json_body()
            .await;

        assert!(third.get("data").is_some() && third.get("errors").is_none());
        assert_eq!(
            subgraphs
                .get_requests_log("reviews/ws")
                .unwrap_or_default()
                .len(),
            2,
            "the first websocket should close after the idle timeout"
        );
        assert!(subgraphs.get_requests_log("reviews").is_none());
    }

    #[ntex::test]
    async fn subgraph_pool_idle_timeout_overrides_the_global_timeout() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                subscriptions:
                    enabled: true
                    websocket:
                        subgraphs:
                            reviews:
                                path: /reviews/ws
                traffic_shaping:
                    all:
                        pool_idle_timeout: 50ms
                        websocket:
                            execute_mode: reuse_existing
                    subgraphs:
                        reviews:
                            pool_idle_timeout: 5s
                    router:
                        dedupe:
                            headers: none
                "#,
            )
            .build()
            .start()
            .await;

        let subscription = router
            .send_graphql_request(
                "subscription { reviewAdded(step: 11, intervalInMs: 0) { id } }",
                None,
                sse_headers(),
            )
            .await;
        assert!(subscription.string_body().await.contains("event: complete"));
        tokio::time::sleep(Duration::from_millis(150)).await;

        let response = router
            .send_graphql_request(REVIEWS_QUERY, None, None)
            .await
            .json_body()
            .await;

        assert!(response.get("data").is_some() && response.get("errors").is_none());
        assert_eq!(
            subgraphs
                .get_requests_log("reviews/ws")
                .unwrap_or_default()
                .len(),
            1,
            "the reviews override should keep the pooled websocket alive"
        );
        assert!(subgraphs.get_requests_log("reviews").is_none());
    }

    #[ntex::test]
    async fn idle_connection_expires_and_query_falls_back_to_http() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                subscriptions:
                    enabled: true
                    websocket:
                        subgraphs:
                            reviews:
                                path: /reviews/ws
                traffic_shaping:
                    all:
                        pool_idle_timeout: 50ms
                        websocket:
                            execute_mode: reuse_existing
                    router:
                        dedupe:
                            headers: none
                "#,
            )
            .build()
            .start()
            .await;

        let subscription = router
            .send_graphql_request(
                "subscription { reviewAdded(step: 11, intervalInMs: 0) { id } }",
                None,
                sse_headers(),
            )
            .await;
        assert!(subscription.string_body().await.contains("event: complete"));
        tokio::time::sleep(Duration::from_millis(150)).await;

        let response = router
            .send_graphql_request(REVIEWS_QUERY, None, None)
            .await
            .json_body()
            .await;

        assert!(response.get("data").is_some());
        assert_eq!(
            subgraphs
                .get_requests_log("reviews")
                .unwrap_or_default()
                .len(),
            1,
            "an expired pool entry should be an HTTP miss"
        );
        assert_eq!(
            subgraphs
                .get_requests_log("reviews/ws")
                .unwrap_or_default()
                .len(),
            1
        );
    }

    #[ntex::test]
    async fn included_headers_isolate_connection_fingerprints() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                subscriptions:
                    enabled: true
                    websocket:
                        subgraphs:
                            reviews:
                                path: /reviews/ws
                traffic_shaping:
                    all:
                        pool_idle_timeout: 5s
                        websocket:
                            execute_mode: reuse_existing
                    router:
                        dedupe:
                            headers:
                                include: [x-tenant]
                "#,
            )
            .build()
            .start()
            .await;
        let tenant = |value: &str| {
            some_header_map! {
                http::header::ACCEPT => "text/event-stream",
                http::header::HeaderName::from_static("x-tenant") => value
            }
        };

        let mut first = router
            .send_graphql_request(LOOPING_SUBSCRIPTION, None, tenant("one"))
            .await;
        first.next().await.unwrap().unwrap();
        let mut second = router
            .send_graphql_request(LOOPING_SUBSCRIPTION, None, tenant("two"))
            .await;
        second.next().await.unwrap().unwrap();

        assert_eq!(
            subgraphs
                .get_requests_log("reviews/ws")
                .unwrap_or_default()
                .len(),
            2,
            "different included identity headers must not share a connection"
        );
    }

    #[ntex::test]
    async fn query_reuses_only_a_matching_selected_header_fingerprint() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                subscriptions:
                    enabled: true
                    websocket:
                        subgraphs:
                            reviews:
                                path: /reviews/ws
                traffic_shaping:
                    all:
                        websocket:
                            execute_mode: reuse_existing
                    router:
                        dedupe:
                            headers:
                                include: [x-tenant]
                "#,
            )
            .build()
            .start()
            .await;
        let headers = |tenant: &'static str, ignored: &'static str, accept| {
            let mut headers = some_header_map! {
                http::header::HeaderName::from_static("x-tenant") => tenant,
                http::header::HeaderName::from_static("x-ignored") => ignored
            }
            .unwrap();
            if accept {
                headers.insert(http::header::ACCEPT, "text/event-stream".parse().unwrap());
            }
            Some(headers)
        };

        let mut subscription = router
            .send_graphql_request(
                LOOPING_SUBSCRIPTION,
                None,
                headers("one", "subscription", true),
            )
            .await;
        subscription.next().await.unwrap().unwrap();

        let mismatch = router
            .send_graphql_request(REVIEWS_QUERY, None, headers("two", "subscription", false))
            .await
            .json_body()
            .await;
        let matching = router
            .send_graphql_request(REVIEWS_QUERY, None, headers("one", "query", false))
            .await
            .json_body()
            .await;

        assert!(mismatch.get("data").is_some() && mismatch.get("errors").is_none());
        assert!(matching.get("data").is_some() && matching.get("errors").is_none());
        assert_eq!(
            subgraphs
                .get_requests_log("reviews")
                .unwrap_or_default()
                .len(),
            1,
            "a different selected header must miss the pool"
        );
        assert_eq!(
            subgraphs
                .get_requests_log("reviews/ws")
                .unwrap_or_default()
                .len(),
            1,
            "unselected headers must not change the connection fingerprint"
        );
    }

    #[ntex::test]
    async fn deduplicated_query_leader_uses_the_initialized_connection() {
        let subgraphs = TestSubgraphs::builder()
            .with_delay(Duration::from_millis(50))
            .build()
            .start()
            .await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .file_config("configs/websocket_pool.yaml")
            .build()
            .start()
            .await;

        let mut subscription = router
            .send_graphql_request(LOOPING_SUBSCRIPTION, None, sse_headers())
            .await;
        subscription.next().await.unwrap().unwrap();

        let first = router.send_graphql_request(REVIEWS_QUERY, None, None);
        let second = router.send_graphql_request(REVIEWS_QUERY, None, None);
        let (first, second) = tokio::join!(first, second);

        let first = first.json_body().await;
        let second = second.json_body().await;
        assert!(first.get("data").is_some() && first.get("errors").is_none());
        assert!(second.get("data").is_some() && second.get("errors").is_none());
        assert_eq!(
            subgraphs
                .get_requests_log("products")
                .unwrap_or_default()
                .len(),
            1,
            "deduplicated followers should not execute their own plan"
        );
        assert!(
            subgraphs.get_requests_log("reviews").is_none(),
            "the deduplication leader should use the initialized websocket"
        );
        assert_eq!(
            subgraphs
                .get_requests_log("reviews/ws")
                .unwrap_or_default()
                .len(),
            1
        );
    }
}
