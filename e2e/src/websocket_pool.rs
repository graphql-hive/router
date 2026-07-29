#[cfg(test)]
mod websocket_pool_e2e_tests {
    use std::time::Duration;

    use futures::StreamExt;
    use sonic_rs::JsonValueTrait;

    use crate::testkit::{some_header_map, ClientResponseExt, TestRouter, TestSubgraphs};

    const POOL_CONFIG: &str = r#"
        supergraph:
            source: file
            path: supergraph.graphql
        subscriptions:
            enabled: true
            websocket:
                all:
                    idle_timeout: 5s
                subgraphs:
                    reviews:
                        path: /reviews/ws
        traffic_shaping:
            router:
                dedupe:
                    headers: none
    "#;

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
            .inline_config(POOL_CONFIG)
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
            .inline_config(POOL_CONFIG)
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
            .inline_config(POOL_CONFIG)
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
            .inline_config(POOL_CONFIG)
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
    async fn queries_and_mutations_never_create_websocket_connections() {
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
    async fn dropping_one_operation_leaves_the_connection_reusable() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(POOL_CONFIG)
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
            .inline_config(POOL_CONFIG.replace("idle_timeout: 5s", "idle_timeout: 50ms"))
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

    #[ntex::test]
    async fn idle_connection_expires_and_query_falls_back_to_http() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(POOL_CONFIG.replace("idle_timeout: 5s", "idle_timeout: 50ms"))
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
            .inline_config(POOL_CONFIG.replace(
                "headers: none",
                "headers:\n                        include: [x-tenant]",
            ))
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
    async fn deduplicated_query_leader_uses_the_initialized_connection() {
        let subgraphs = TestSubgraphs::builder()
            .with_delay(Duration::from_millis(50))
            .build()
            .start()
            .await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(POOL_CONFIG)
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
