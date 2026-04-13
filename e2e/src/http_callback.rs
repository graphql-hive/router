#[cfg(test)]
mod http_callback_e2e_tests {
    use futures::StreamExt;
    use ntex::http;
    use sonic_rs::{json, JsonValueTrait};

    use crate::{
        flakey,
        testkit::{some_header_map, ClientResponseExt, TestRouter, TestSubgraphs},
    };

    #[ntex::test]
    async fn listen_on_different_port() {
        let subgraphs = TestSubgraphs::builder().build().start().await;

        // on slow systems when running tests concurrently, the available
        // port might become unavailable by the time the router starts and binds
        // the callback handler to it, causing the test to fail. in order to avoid
        // we use a fixed high port that is unlikely to be used by other processes
        // and cause conflicts, or get allocated (OS starts with 50000)
        let callback_port = 61000;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            // .with_port() router is on a different port than the callback listener anyways
            .inline_config(format!(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                subscriptions:
                    enabled: true
                    callback:
                        listen: 0.0.0.0:{callback_port}
                        public_url: http://0.0.0.0:{callback_port}/callback
                        subgraphs:
                            - reviews
                "#
            ))
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(
                r#"
                subscription {
                    reviewAdded(intervalInMs: 0) {
                        id
                        product {
                            name
                        }
                    }
                }
                "#,
                None,
                some_header_map!(
                    http::header::ACCEPT => "text/event-stream",
                ),
            )
            .await;

        assert_eq!(res.status(), 200, "Expected 200 OK");

        let body = res.string_body().await;

        assert!(
            body.contains(
                r#"data: {"data":{"reviewAdded":{"id":"1","product":{"name":"Table"}}}}"#
            ),
            "Expected at least one emitted event, got: {}",
            body
        );
        assert!(body.contains("event: complete"));
    }

    #[ntex::test]
    async fn complete_active_subscription_on_heartbeat_timeout() {
        let subgraphs = TestSubgraphs::builder().build().start().await;

        let router_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let router_port = router_listener.local_addr().unwrap().port();
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .with_listener(router_listener)
            .inline_config(format!(
                r#"
                    supergraph:
                        source: file
                        path: supergraph.graphql
                    headers:
                        all:
                            request:
                                - propagate:
                                    named: x-disable-http-callback-heartbeats
                    subscriptions:
                        enabled: true
                        callback:
                            heartbeat_interval: 500ms
                            public_url: http://0.0.0.0:{router_port}/callback
                            subgraphs:
                                - reviews
                    "#
            ))
            .build()
            .start()
            .await;

        let res = router
                .send_graphql_request(
                    r#"
                    subscription {
                        reviewAdded(
                            # emitted messages do not count as heartbeats
                            intervalInMs: 300
                        ) {
                            id
                            product {
                                name
                            }
                        }
                    }
                    "#,
                    None,
                    some_header_map!(
                        http::header::ACCEPT => "text/event-stream",
                        http::header::HeaderName::from_static("x-disable-http-callback-heartbeats") => "true"
                    ),
                )
                .await;

        assert_eq!(res.status(), 200, "Expected 200 OK");

        let body = res.string_body().await;

        // emitted at least one event
        assert!(
            body.contains(
                r#"data: {"data":{"reviewAdded":{"id":"1","product":{"name":"Table"}}}}"#
            ),
            "Expected at least one emitted event, got: {}",
            body
        );

        // kicked off client, eventually
        assert!(body.contains(r#"{"data":null,"errors":[{"message":"Subgraph gone due to heartbeat timeout","extensions":{"code":"SUBGRAPH_GONE"}}]}"#));

        // completed stream
        assert!(body.contains("event: complete"));
    }

    #[ntex::test]
    async fn client_disconnect_removes_subscription() {
        let subgraphs = TestSubgraphs::builder().build().start().await;

        let router_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let router_port = router_listener.local_addr().unwrap().port();
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .with_listener(router_listener)
            .inline_config(format!(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                subscriptions:
                    enabled: true
                    callback:
                        public_url: http://0.0.0.0:{router_port}/callback
                        subgraphs:
                            - reviews
                "#
            ))
            .build()
            .start()
            .await;

        // Use a longer interval so we have time to cancel before the stream completes
        let mut res = router
            .send_graphql_request(
                r#"
                subscription {
                    reviewAdded(intervalInMs: 100) {
                        id
                    }
                }
                "#,
                None,
                some_header_map!(
                    http::header::ACCEPT => "text/event-stream"
                ),
            )
            .await;

        assert_eq!(res.status(), 200, "Expected 200 OK");

        // Read first chunk to ensure the subscription is active
        let chunk_bytes = res.next().await.unwrap().unwrap();
        let chunk_str = std::str::from_utf8(&chunk_bytes).unwrap();
        assert!(
            chunk_str.contains(r#"{"data":{"reviewAdded":{"id":"1"}}}"#),
            "Expected first emission, got: {}",
            chunk_str
        );

        // Extract subscriptionId and verifier from the request the router sent to the subgraph
        let subgraph_requests = subgraphs
            .get_requests_log("reviews")
            .expect("expected requests sent to reviews subgraph");
        let body_bytes = subgraph_requests[0]
            .body
            .as_ref()
            .expect("expected request body");
        let body_json: sonic_rs::Value =
            sonic_rs::from_slice(body_bytes).expect("expected valid JSON body");
        let subscription_id = body_json["extensions"]["subscription"]["subscriptionId"]
            .as_str()
            .expect("expected subscriptionId in request extensions")
            .to_string();
        let verifier = body_json["extensions"]["subscription"]["verifier"]
            .as_str()
            .expect("expected verifier in request extensions")
            .to_string();

        // Disconnect the client — this should propagate to the router and remove the subscription
        drop(res);

        // Give the router a moment to process the disconnect and clean up
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // A check for the now-removed subscription should return 404
        let check_res = router
            .serv()
            .post(format!("/callback/{subscription_id}"))
            .set_header("subscription-protocol", "callback/1.0")
            .send_json(&json!({
                "kind": "subscription",
                "action": "check",
                "id": subscription_id,
                "verifier": verifier,
            }))
            .await
            .expect("failed to send callback check request");

        assert_eq!(
            check_res.status(),
            404,
            "Expected 404 after client disconnect removed the subscription"
        );
    }

    #[ntex::test]
    async fn invalid_verifier_is_rejected() {
        let subgraphs = TestSubgraphs::builder().build().start().await;

        let router_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let router_port = router_listener.local_addr().unwrap().port();
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .with_listener(router_listener)
            .inline_config(format!(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                subscriptions:
                    enabled: true
                    callback:
                        public_url: http://0.0.0.0:{router_port}/callback
                        subgraphs:
                            - reviews
                "#
            ))
            .build()
            .start()
            .await;

        let mut res = router
            .send_graphql_request(
                r#"
                subscription {
                    reviewAdded(intervalInMs: 100) {
                        id
                    }
                }
                "#,
                None,
                some_header_map!(
                    http::header::ACCEPT => "text/event-stream"
                ),
            )
            .await;

        assert_eq!(res.status(), 200, "Expected 200 OK");

        let chunk_bytes = res.next().await.unwrap().unwrap();
        let chunk_str = std::str::from_utf8(&chunk_bytes).unwrap();
        assert!(
            chunk_str.contains(r#"{"data":{"reviewAdded":{"id":"1"}}}"#),
            "Expected first emission, got: {}",
            chunk_str
        );

        let subgraph_requests = subgraphs
            .get_requests_log("reviews")
            .expect("expected requests sent to reviews subgraph");
        let body_bytes = subgraph_requests[0]
            .body
            .as_ref()
            .expect("expected request body");
        let body_json: sonic_rs::Value =
            sonic_rs::from_slice(body_bytes).expect("expected valid JSON body");
        let subscription_id = body_json["extensions"]["subscription"]["subscriptionId"]
            .as_str()
            .expect("expected subscriptionId in request extensions")
            .to_string();

        // Send a check with a verifier that doesn't match the subscription's verifier
        let wrong_verifier = "this-is-not-the-correct-verifier";
        let check_res = router
            .serv()
            .post(format!("/callback/{subscription_id}"))
            .set_header("subscription-protocol", "callback/1.0")
            .send_json(&json!({
                "kind": "subscription",
                "action": "check",
                "id": subscription_id,
                "verifier": wrong_verifier,
            }))
            .await
            .expect("failed to send callback check request");

        assert_eq!(
            check_res.status(),
            400,
            "Expected 400 when using an invalid verifier"
        );

        // keep alive until the end of the test so the subscription stays active
        drop(res);
    }
}
