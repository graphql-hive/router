#[cfg(test)]
mod subscriptions_e2e_tests {

    use insta::assert_snapshot;
    use ntex::http;
    use reqwest::StatusCode;
    use sonic_rs::json;

    use crate::testkit::{
        some_header_map, ResponseLike, TestRouterBuilder, TestSubgraphsBuilder,
    };

    #[ntex::test]
    async fn subscription_not_allowed_when_disabled() {
        let subgraphs = TestSubgraphsBuilder::new().build().start().await;
        let router = TestRouterBuilder::new()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                # disabled by default
                # subscriptions:
                #     enabled: false
                "#,
            )
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(
                r#"
                subscription {
                    reviewAdded(intervalInMs: 0) {
                        product {
                            upc
                        }
                    }
                }
                "#,
                None,
                some_header_map! {
                    http::header::ACCEPT => "text/event-stream"
                },
            )
            .await;

        // even though subscriptions are disabled, we accept the stream
        assert_eq!(res.status(), 200, "Expected 200 OK");

        let body = res.body().await.unwrap();
        let body_str = std::str::from_utf8(&body).unwrap();

        assert_snapshot!(body_str, @r#"
        event: next
        data: {"errors":[{"message":"Subscriptions are not supported","extensions":{"code":"SUBSCRIPTIONS_NOT_SUPPORTED"}}]}

        event: complete
        "#);
    }

    #[ntex::test]
    async fn subscription_no_entity_resolution_sse_subgraph() {
        let subgraphs = TestSubgraphsBuilder::new()
            .with_subscriptions_protocol(subgraphs::SubscriptionProtocol::SseOnly)
            .build()
            .start()
            .await;
        let router = TestRouterBuilder::new()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                subscriptions:
                    enabled: true
                "#,
            )
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(
                r#"
                subscription {
                    reviewAdded(intervalInMs: 0) {
                        product {
                            upc
                        }
                    }
                }
                "#,
                None,
                some_header_map! {
                    http::header::ACCEPT => "text/event-stream"
                },
            )
            .await;

        assert_eq!(res.status(), 200, "Expected 200 OK");

        let content_type_header = res
            .header("content-type")
            .expect("must have content-type header");

        let body = res.body().await.unwrap();
        let body_str = std::str::from_utf8(&body).unwrap();

        assert_snapshot!(body_str, @r#"
        event: next
        data: {"data":{"reviewAdded":{"product":{"upc":"1"}}}}

        event: next
        data: {"data":{"reviewAdded":{"product":{"upc":"1"}}}}

        event: next
        data: {"data":{"reviewAdded":{"product":{"upc":"1"}}}}

        event: next
        data: {"data":{"reviewAdded":{"product":{"upc":"1"}}}}

        event: next
        data: {"data":{"reviewAdded":{"product":{"upc":"2"}}}}

        event: next
        data: {"data":{"reviewAdded":{"product":{"upc":"2"}}}}

        event: next
        data: {"data":{"reviewAdded":{"product":{"upc":"2"}}}}

        event: next
        data: {"data":{"reviewAdded":{"product":{"upc":"2"}}}}

        event: next
        data: {"data":{"reviewAdded":{"product":{"upc":"3"}}}}

        event: next
        data: {"data":{"reviewAdded":{"product":{"upc":"4"}}}}

        event: next
        data: {"data":{"reviewAdded":{"product":{"upc":"4"}}}}

        event: complete
        "#);

        // we check this at the end because the body will hold clues to why the test fails
        assert_eq!(
            content_type_header, "text/event-stream",
            "Expected Content-Type to be text/event-stream"
        );
    }

    #[ntex::test]
    async fn subscription_no_entity_resolution_multipart_subgraph() {
        let subgraphs = TestSubgraphsBuilder::new()
            .with_subscriptions_protocol(subgraphs::SubscriptionProtocol::MultipartOnly)
            .build()
            .start()
            .await;
        let router = TestRouterBuilder::new()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                subscriptions:
                    enabled: true
                "#,
            )
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(
                r#"
                subscription {
                    reviewAdded(intervalInMs: 0) {
                        product {
                            upc
                        }
                    }
                }
                "#,
                None,
                some_header_map! {
                    http::header::ACCEPT => "text/event-stream"
                },
            )
            .await;

        assert!(res.status().is_success(), "Expected 200 OK");

        let content_type_header = res
            .header("content-type")
            .expect("must have content-type header");

        let body = res.body().await.unwrap();
        let body_str = std::str::from_utf8(&body).unwrap();

        assert_snapshot!(body_str, @r#"
        event: next
        data: {"data":{"reviewAdded":{"product":{"upc":"1"}}}}

        event: next
        data: {"data":{"reviewAdded":{"product":{"upc":"1"}}}}

        event: next
        data: {"data":{"reviewAdded":{"product":{"upc":"1"}}}}

        event: next
        data: {"data":{"reviewAdded":{"product":{"upc":"1"}}}}

        event: next
        data: {"data":{"reviewAdded":{"product":{"upc":"2"}}}}

        event: next
        data: {"data":{"reviewAdded":{"product":{"upc":"2"}}}}

        event: next
        data: {"data":{"reviewAdded":{"product":{"upc":"2"}}}}

        event: next
        data: {"data":{"reviewAdded":{"product":{"upc":"2"}}}}

        event: next
        data: {"data":{"reviewAdded":{"product":{"upc":"3"}}}}

        event: next
        data: {"data":{"reviewAdded":{"product":{"upc":"4"}}}}

        event: next
        data: {"data":{"reviewAdded":{"product":{"upc":"4"}}}}

        event: complete
        "#);

        // we check this at the end because the body will hold clues to why the test fails
        assert_eq!(
            content_type_header, "text/event-stream",
            "Expected Content-Type to be text/event-stream"
        );
    }

    #[ntex::test]
    async fn subscription_yes_entity_resolution() {
        let subgraphs = TestSubgraphsBuilder::new().build().start().await;
        let router = TestRouterBuilder::new()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                subscriptions:
                    enabled: true
                "#,
            )
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
                some_header_map! {
                    http::header::ACCEPT => "text/event-stream"
                },
            )
            .await;

        assert!(res.status().is_success(), "Expected 200 OK");

        let content_type_header = res
            .header("content-type")
            .expect("must have content-type header");

        let body = res.body().await.unwrap();
        let body_str = std::str::from_utf8(&body).unwrap();

        assert_snapshot!(body_str, @r#"
        event: next
        data: {"data":{"reviewAdded":{"id":"1","product":{"name":"Table"}}}}

        event: next
        data: {"data":{"reviewAdded":{"id":"2","product":{"name":"Table"}}}}

        event: next
        data: {"data":{"reviewAdded":{"id":"3","product":{"name":"Table"}}}}

        event: next
        data: {"data":{"reviewAdded":{"id":"4","product":{"name":"Table"}}}}

        event: next
        data: {"data":{"reviewAdded":{"id":"5","product":{"name":"Couch"}}}}

        event: next
        data: {"data":{"reviewAdded":{"id":"6","product":{"name":"Couch"}}}}

        event: next
        data: {"data":{"reviewAdded":{"id":"7","product":{"name":"Couch"}}}}

        event: next
        data: {"data":{"reviewAdded":{"id":"8","product":{"name":"Couch"}}}}

        event: next
        data: {"data":{"reviewAdded":{"id":"9","product":{"name":"Glass"}}}}

        event: next
        data: {"data":{"reviewAdded":{"id":"10","product":{"name":"Chair"}}}}

        event: next
        data: {"data":{"reviewAdded":{"id":"11","product":{"name":"Chair"}}}}

        event: complete
        "#);

        // we check this at the end because the body will hold clues to why the test fails
        assert_eq!(
            content_type_header, "text/event-stream",
            "Expected Content-Type to be text/event-stream"
        );
    }

    #[ntex::test]
    async fn subscription_yes_entity_resolution_multipart_client() {
        let subgraphs = TestSubgraphsBuilder::new().build().start().await;
        let router = TestRouterBuilder::new()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                subscriptions:
                    enabled: true
                "#,
            )
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
                some_header_map! {
                    // as per https://www.apollographql.com/docs/graphos/routing/operations/subscriptions/multipart-protocol#executing-a-subscription
                    http::header::ACCEPT => r#"multipart/mixed;subscriptionSpec="1.0", application/json"#
                },
            )
            .await;

        assert!(res.status().is_success(), "Expected 200 OK");

        let content_type_header = res
            .header("content-type")
            .expect("must have content-type header");

        let body = res.body().await.unwrap();
        let body_str = std::str::from_utf8(&body).unwrap();

        assert_snapshot!(body_str, @r#"
        --graphql
        Content-Type: application/json

        {"payload":{"data":{"reviewAdded":{"id":"1","product":{"name":"Table"}}}}}
        --graphql
        Content-Type: application/json

        {"payload":{"data":{"reviewAdded":{"id":"2","product":{"name":"Table"}}}}}
        --graphql
        Content-Type: application/json

        {"payload":{"data":{"reviewAdded":{"id":"3","product":{"name":"Table"}}}}}
        --graphql
        Content-Type: application/json

        {"payload":{"data":{"reviewAdded":{"id":"4","product":{"name":"Table"}}}}}
        --graphql
        Content-Type: application/json

        {"payload":{"data":{"reviewAdded":{"id":"5","product":{"name":"Couch"}}}}}
        --graphql
        Content-Type: application/json

        {"payload":{"data":{"reviewAdded":{"id":"6","product":{"name":"Couch"}}}}}
        --graphql
        Content-Type: application/json

        {"payload":{"data":{"reviewAdded":{"id":"7","product":{"name":"Couch"}}}}}
        --graphql
        Content-Type: application/json

        {"payload":{"data":{"reviewAdded":{"id":"8","product":{"name":"Couch"}}}}}
        --graphql
        Content-Type: application/json

        {"payload":{"data":{"reviewAdded":{"id":"9","product":{"name":"Glass"}}}}}
        --graphql
        Content-Type: application/json

        {"payload":{"data":{"reviewAdded":{"id":"10","product":{"name":"Chair"}}}}}
        --graphql
        Content-Type: application/json

        {"payload":{"data":{"reviewAdded":{"id":"11","product":{"name":"Chair"}}}}}
        --graphql--
        "#);

        // we check this at the end because the body will hold clues to why the test fails
        assert_eq!(
            content_type_header, "multipart/mixed;boundary=graphql",
            "Expected Content-Type to be multipart/mixed; boundary=graphql"
        );
    }

    #[ntex::test]
    async fn subscription_yes_entity_resolution_websocket_subgraph() {
        let subgraphs = TestSubgraphsBuilder::new().build().start().await;
        let router = TestRouterBuilder::new()
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
                "#,
            )
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
                some_header_map! {
                    http::header::ACCEPT => "text/event-stream"
                },
            )
            .await;

        assert!(res.status().is_success(), "Expected 200 OK");

        let body = res.body().await.unwrap();
        let body_str = std::str::from_utf8(&body).unwrap();

        assert_snapshot!(body_str, @r#"
        event: next
        data: {"data":{"reviewAdded":{"id":"1","product":{"name":"Table"}}}}

        event: next
        data: {"data":{"reviewAdded":{"id":"2","product":{"name":"Table"}}}}

        event: next
        data: {"data":{"reviewAdded":{"id":"3","product":{"name":"Table"}}}}

        event: next
        data: {"data":{"reviewAdded":{"id":"4","product":{"name":"Table"}}}}

        event: next
        data: {"data":{"reviewAdded":{"id":"5","product":{"name":"Couch"}}}}

        event: next
        data: {"data":{"reviewAdded":{"id":"6","product":{"name":"Couch"}}}}

        event: next
        data: {"data":{"reviewAdded":{"id":"7","product":{"name":"Couch"}}}}

        event: next
        data: {"data":{"reviewAdded":{"id":"8","product":{"name":"Couch"}}}}

        event: next
        data: {"data":{"reviewAdded":{"id":"9","product":{"name":"Glass"}}}}

        event: next
        data: {"data":{"reviewAdded":{"id":"10","product":{"name":"Chair"}}}}

        event: next
        data: {"data":{"reviewAdded":{"id":"11","product":{"name":"Chair"}}}}

        event: complete
        "#);
    }

    #[ntex::test]
    async fn subscription_entity_resolution_with_requires() {
        let subgraphs = TestSubgraphsBuilder::new().build().start().await;
        let router = TestRouterBuilder::new()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                subscriptions:
                    enabled: true
                "#,
            )
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(
                r#"
                subscription {
                    reviewAdded(intervalInMs: 0) {
                        product {
                            name
                            shippingEstimate
                        }
                    }
                }
                "#,
                None,
                some_header_map! {
                    http::header::ACCEPT => "text/event-stream"
                },
            )
            .await;

        assert!(res.status().is_success(), "Expected 200 OK");

        let content_type_header = res
            .header("content-type")
            .expect("must have content-type header");

        let body = res.body().await.unwrap();
        let body_str = std::str::from_utf8(&body).unwrap();

        assert_snapshot!(body_str, @r#"
        event: next
        data: {"data":{"reviewAdded":{"product":{"name":"Table","shippingEstimate":50}}}}

        event: next
        data: {"data":{"reviewAdded":{"product":{"name":"Table","shippingEstimate":50}}}}

        event: next
        data: {"data":{"reviewAdded":{"product":{"name":"Table","shippingEstimate":50}}}}

        event: next
        data: {"data":{"reviewAdded":{"product":{"name":"Table","shippingEstimate":50}}}}

        event: next
        data: {"data":{"reviewAdded":{"product":{"name":"Couch","shippingEstimate":0}}}}

        event: next
        data: {"data":{"reviewAdded":{"product":{"name":"Couch","shippingEstimate":0}}}}

        event: next
        data: {"data":{"reviewAdded":{"product":{"name":"Couch","shippingEstimate":0}}}}

        event: next
        data: {"data":{"reviewAdded":{"product":{"name":"Couch","shippingEstimate":0}}}}

        event: next
        data: {"data":{"reviewAdded":{"product":{"name":"Glass","shippingEstimate":10}}}}

        event: next
        data: {"data":{"reviewAdded":{"product":{"name":"Chair","shippingEstimate":50}}}}

        event: next
        data: {"data":{"reviewAdded":{"product":{"name":"Chair","shippingEstimate":50}}}}

        event: complete
        "#);

        // we check this at the end because the body will hold clues to why the test fails
        assert_eq!(
            content_type_header, "text/event-stream",
            "Expected Content-Type to be text/event-stream"
        );
    }

    #[ntex::test]
    async fn subscription_with_variable_forwarding() {
        let subgraphs = TestSubgraphsBuilder::new().build().start().await;
        let router = TestRouterBuilder::new()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                subscriptions:
                    enabled: true
                "#,
            )
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(
                r#"
                subscription ($upc: String!) {
                    reviewAddedForProduct(productUpc: $upc, intervalInMs: 0) {
                        product {
                            upc
                            name
                        }
                    }
                }
                "#,
                Some(json!({
                    "upc": "2"
                })),
                some_header_map! {
                    http::header::ACCEPT => "text/event-stream"
                },
            )
            .await;

        assert!(res.status().is_success(), "Expected 200 OK");

        let content_type_header = res
            .header("content-type")
            .expect("must have content-type header");

        let body = res.body().await.unwrap();
        let body_str = std::str::from_utf8(&body).unwrap();

        assert_snapshot!(body_str, @r#"
        event: next
        data: {"data":{"reviewAddedForProduct":{"product":{"upc":"2","name":"Couch"}}}}

        event: next
        data: {"data":{"reviewAddedForProduct":{"product":{"upc":"2","name":"Couch"}}}}

        event: next
        data: {"data":{"reviewAddedForProduct":{"product":{"upc":"2","name":"Couch"}}}}

        event: next
        data: {"data":{"reviewAddedForProduct":{"product":{"upc":"2","name":"Couch"}}}}

        event: complete
        "#);

        // we check this at the end because the body will hold clues to why the test fails
        assert_eq!(
            content_type_header, "text/event-stream",
            "Expected Content-Type to be text/event-stream"
        );
    }

    #[ntex::test]
    async fn subscription_http_accept_multipart_and_sse() {
        let subgraphs = TestSubgraphsBuilder::new().build().start().await;
        let router = TestRouterBuilder::new()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                subscriptions:
                    enabled: true
                "#,
            )
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(
                r#"
                subscription ($upc: String!) {
                    reviewAddedForProduct(productUpc: $upc, intervalInMs: 0) {
                        product {
                            upc
                            name
                        }
                    }
                }
                "#,
                Some(json!({
                    "upc": "2"
                })),
                some_header_map! {
                    http::header::ACCEPT => "text/event-stream"
                },
            )
            .await;

        assert!(res.status().is_success(), "Expected 200 OK");

        let subgraph_request = subgraphs
            .get_requests_log("reviews")
            .expect("expected requests sent to reviews subgraph");

        let Ok(accept_header) = subgraph_request
            .get(0)
            .expect("expected at least one request to reviews")
            .headers
            .get("accept")
            .expect("expected accept header to be sent with the subgraph request")
            .to_str()
        else {
            panic!("accept header could not be converted to string")
        };

        assert_snapshot!(accept_header, @r#"multipart/mixed;subscriptionSpec="1.0", text/event-stream"#);
    }

    #[ntex::test]
    async fn subscription_stream_failed_source_subgraph_requests() {
        let subgraphs = TestSubgraphsBuilder::new()
            .with_on_request(|_req| {
                Some(ResponseLike::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    None,
                    None,
                ))
            })
            .build()
            .start()
            .await;

        let router = TestRouterBuilder::new()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                subscriptions:
                    enabled: true
                "#,
            )
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(
                r#"
                subscription {
                    reviewAdded(intervalInMs: 0) {
                        id
                    }
                }
                "#,
                None,
                some_header_map! {
                    http::header::ACCEPT => "text/event-stream"
                },
            )
            .await;

        assert_eq!(res.status(), 200, "Expected 200 OK");

        let body = res.body().await.unwrap();
        let body_str = std::str::from_utf8(&body).unwrap();

        assert!(
            body_str.contains(r#"{"code":"SUBGRAPH_REQUEST_FAILURE"}"#),
            "Expected '{}' to contain the subgraph request failure error code",
            body_str
        );
    }

    #[ntex::test]
    async fn subscription_stream_failed_entity_resolution_requests() {
        let subgraphs = TestSubgraphsBuilder::new()
            .with_on_request(|req| {
                if req.path.contains("products") {
                    // entity resolution
                    Some(ResponseLike::new(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Some(
                            json!({
                                "errors": [{"message": "Something Went Wrong!"}]
                            })
                            .to_string(),
                        ),
                        some_header_map! {
                            http::header::CONTENT_TYPE => "application/json"
                        },
                    ))
                } else {
                    // subscription itself (on "reviews" subgraph)
                    None
                }
            })
            .build()
            .start()
            .await;

        let router = TestRouterBuilder::new()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                subscriptions:
                    enabled: true
                "#,
            )
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(
                r#"
                subscription ($upc: String!) {
                    reviewAddedForProduct(productUpc: $upc, intervalInMs: 0) {
                        product {
                            name
                        }
                    }
                }
                "#,
                Some(json!({
                    "upc": "2"
                })),
                some_header_map! {
                    http::header::ACCEPT => "text/event-stream"
                },
            )
            .await;

        let body = res.body().await.unwrap();
        let body_str = std::str::from_utf8(&body).unwrap();

        assert_snapshot!(body_str, @r#"
        event: next
        data: {"data":{"reviewAddedForProduct":{"product":{"name":null}}},"errors":[{"message":"Something Went Wrong!","extensions":{"code":"DOWNSTREAM_SERVICE_ERROR","serviceName":"products","affectedPath":"reviewAddedForProduct.product"}}]}

        event: next
        data: {"data":{"reviewAddedForProduct":{"product":{"name":null}}},"errors":[{"message":"Something Went Wrong!","extensions":{"code":"DOWNSTREAM_SERVICE_ERROR","serviceName":"products","affectedPath":"reviewAddedForProduct.product"}}]}

        event: next
        data: {"data":{"reviewAddedForProduct":{"product":{"name":null}}},"errors":[{"message":"Something Went Wrong!","extensions":{"code":"DOWNSTREAM_SERVICE_ERROR","serviceName":"products","affectedPath":"reviewAddedForProduct.product"}}]}

        event: next
        data: {"data":{"reviewAddedForProduct":{"product":{"name":null}}},"errors":[{"message":"Something Went Wrong!","extensions":{"code":"DOWNSTREAM_SERVICE_ERROR","serviceName":"products","affectedPath":"reviewAddedForProduct.product"}}]}

        event: complete
        "#);
    }

    #[ntex::test]
    async fn subscription_stream_client_cancelled() {
        use futures::StreamExt;

        let subgraphs = TestSubgraphsBuilder::new().build().start().await;
        let router = TestRouterBuilder::new()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                subscriptions:
                    enabled: true
                "#,
            )
            .build()
            .start()
            .await;

        // Use a longer interval so we have time to cancel
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
                some_header_map! {
                    http::header::ACCEPT => "text/event-stream"
                },
            )
            .await;

        assert!(res.status().is_success(), "Expected 200 OK");

        // read first chunk
        let chunk_bytes = res.next().await.unwrap().unwrap();
        let chunk_str = std::str::from_utf8(&chunk_bytes).unwrap();

        assert_snapshot!(chunk_str, @r#"
        event: next
        data: {"data":{"reviewAdded":{"id":"1"}}}
        "#);

        // read second chunk to ensure stream is flowing
        let chunk_bytes = res.next().await.unwrap().unwrap();
        let chunk_str = std::str::from_utf8(&chunk_bytes).unwrap();

        assert_snapshot!(chunk_str, @r#"
        event: next
        data: {"data":{"reviewAdded":{"id":"2"}}}
        "#);

        // cancel
        drop(res);

        // TODO: check if propagated?
    }

    #[ntex::test]
    async fn subscription_header_propagation_for_subscription() {
        let subgraphs = TestSubgraphsBuilder::new().build().start().await;
        let router = TestRouterBuilder::new()
            .with_subgraphs(&subgraphs)
            .file_config("configs/header_propagation.router.yaml")
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(
                r#"
                subscription {
                    reviewAdded(intervalInMs: 0) {
                        id
                    }
                }
                "#,
                None,
                some_header_map! {
                    http::header::ACCEPT => "text/event-stream",
                    http::header::HeaderName::from_static("x-context") => "maybe-propagate"
                },
            )
            .await;

        assert_eq!(res.status(), 200, "Expected 200 OK");

        // we have to consume the body to ensure the subscription is fully processed
        let body = res.body().await.unwrap();
        std::str::from_utf8(&body).unwrap();

        let subgraph_requests = subgraphs
            .get_requests_log("reviews")
            .expect("expected requests sent to reviews subgraph");

        let context_header = subgraph_requests[0]
            .headers
            .get("x-context")
            .expect("expected x-context header to be present");

        assert_eq!(
            context_header, "maybe-propagate",
            "expected x-context header to be propagated to subgraph"
        );
    }

    #[ntex::test]
    async fn subscription_header_propagation_for_entity_resolution() {
        let subgraphs = TestSubgraphsBuilder::new().build().start().await;
        let router = TestRouterBuilder::new()
            .with_subgraphs(&subgraphs)
            .file_config("configs/header_propagation.router.yaml")
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(
                r#"
                subscription {
                    reviewAdded(intervalInMs: 0) {
                        product {
                            name
                        }
                    }
                }
                "#,
                None,
                some_header_map! {
                    http::header::ACCEPT => "text/event-stream",
                    http::header::HeaderName::from_static("x-context") => "maybe-propagate"
                },
            )
            .await;

        assert_eq!(res.status(), 200, "Expected 200 OK");

        // we have to consume the body to ensure all entity resolutions were made
        let body = res.body().await.unwrap();
        std::str::from_utf8(&body).unwrap();

        let subgraph_requests = subgraphs
            .get_requests_log("products")
            .expect("expected requests sent to products subgraph");

        // every entity resolution request must have the propagated header
        for subgraph_request in subgraph_requests {
            let context_header = subgraph_request
                .headers
                .get("x-context")
                .expect("expected x-context header to be present");

            assert_eq!(
                context_header, "maybe-propagate",
                "expected x-context header to be propagated to subgraph"
            );
        }
    }

    #[ntex::test]
    async fn subscription_propagate_connection_termination_subgraph() {
        let subgraphs = TestSubgraphsBuilder::new().build().start().await;
        let router = TestRouterBuilder::new()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                subscriptions:
                    enabled: true
                headers:
                    all:
                        request:
                            - propagate:
                                named: x-break-after
                "#,
            )
            .build()
            .start()
            .await;

        // NOTE: we add a 10ms interval because providing 0 will end the connection while the buffer is still being written to leading to a different error
        let res = router
            .send_graphql_request(
                r#"
                subscription {
                    reviewAdded(intervalInMs: 10) {
                        id
                    }
                }
                "#,
                None,
                some_header_map! {
                    http::header::ACCEPT => "text/event-stream",
                    http::header::HeaderName::from_static("x-break-after") => "3"
                },
            )
            .await;

        let body = res.body().await.unwrap();
        let body_str = std::str::from_utf8(&body).unwrap();

        assert_snapshot!(body_str, @r#"
        event: next
        data: {"data":{"reviewAdded":{"id":"1"}}}

        event: next
        data: {"data":{"reviewAdded":{"id":"2"}}}

        event: next
        data: {"data":{"reviewAdded":{"id":"3"}}}

        event: next
        data: {"data":null,"errors":[{"message":"Failed to execute request to subgraph","extensions":{"code":"SUBGRAPH_SUBSCRIPTION_STREAM_ERROR","serviceName":"reviews"}}]}

        event: complete
        "#);
    }
}
