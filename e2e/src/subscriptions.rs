// TODO: when query fails for whatever reason and the client is requesting an SSE, it MUST be in the stream

#[cfg(test)]
mod subscription_e2e_tests {
    use std::sync::Arc;

    use insta::assert_snapshot;
    use ntex::{http, web::test};
    use reqwest::StatusCode;
    use sonic_rs::json;
    use subgraphs::InterceptedResponse;

    use crate::testkit::{
        init_graphql_request, init_router_from_config_inline, wait_for_readiness, SubgraphsServer,
    };

    fn get_content_type_header(res: &ntex::web::WebResponse) -> String {
        res.headers()
            .get(ntex::http::header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap()
            .to_string()
    }

    #[ntex::test]
    async fn subscription_no_entity_resolution_sse_subgraph() {
        let _subgraphs_server = SubgraphsServer::start_with_subscriptions_protocol(
            subgraphs::SubscriptionProtocol::SseOnly,
        )
        .await;

        let router = init_router_from_config_inline(&format!(
            r#"
            supergraph:
                source: file
                path: supergraph.graphql
            "#
        ))
        .await
        .unwrap();

        wait_for_readiness(&router.app).await;

        let req = init_graphql_request(
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
        )
        .header(http::header::ACCEPT, "text/event-stream")
        .to_request();

        let res = test::call_service(&router.app, req).await;

        assert!(res.status().is_success(), "Expected 200 OK");

        let content_type_header = get_content_type_header(&res);

        let body = test::read_body(res).await;
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
        let _subgraphs_server = SubgraphsServer::start_with_subscriptions_protocol(
            subgraphs::SubscriptionProtocol::MultipartOnly,
        )
        .await;

        let router = init_router_from_config_inline(&format!(
            r#"
            supergraph:
                source: file
                path: supergraph.graphql
            "#
        ))
        .await
        .unwrap();

        wait_for_readiness(&router.app).await;

        let req = init_graphql_request(
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
        )
        .header(http::header::ACCEPT, "text/event-stream")
        .to_request();

        let res = test::call_service(&router.app, req).await;

        assert!(res.status().is_success(), "Expected 200 OK");

        let content_type_header = get_content_type_header(&res);

        let body = test::read_body(res).await;
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
        let _subgraphs_server = SubgraphsServer::start().await;

        let router = init_router_from_config_inline(&format!(
            r#"
            supergraph:
                source: file
                path: supergraph.graphql
            "#
        ))
        .await
        .unwrap();

        wait_for_readiness(&router.app).await;

        let req = init_graphql_request(
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
        )
        .header(http::header::ACCEPT, "text/event-stream")
        .to_request();

        let res = test::call_service(&router.app, req).await;

        assert!(res.status().is_success(), "Expected 200 OK");

        let content_type_header = get_content_type_header(&res);

        let body = test::read_body(res).await;
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
        let _subgraphs_server = SubgraphsServer::start().await;

        let router = init_router_from_config_inline(&format!(
            r#"
            supergraph:
                source: file
                path: supergraph.graphql
            "#
        ))
        .await
        .unwrap();

        wait_for_readiness(&router.app).await;

        let req = init_graphql_request(
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
        )
        .header(
            http::header::ACCEPT,
            // as per https://www.apollographql.com/docs/graphos/routing/operations/subscriptions/multipart-protocol#executing-a-subscription
            r#"multipart/mixed;subscriptionSpec="1.0", application/json"#,
        )
        .to_request();

        let res = test::call_service(&router.app, req).await;

        assert!(res.status().is_success(), "Expected 200 OK");

        let content_type_header = get_content_type_header(&res);

        let body = test::read_body(res).await;
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
            "Expected Content-Type to be multipart/mixed;boundary=graphql"
        );
    }

    #[ntex::test]
    async fn subscription_entity_resolution_with_requires() {
        let _subgraphs_server = SubgraphsServer::start().await;

        let router = init_router_from_config_inline(&format!(
            r#"
            supergraph:
                source: file
                path: supergraph.graphql
            "#
        ))
        .await
        .unwrap();

        wait_for_readiness(&router.app).await;

        let req = init_graphql_request(
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
        )
        .header(http::header::ACCEPT, "text/event-stream")
        .to_request();

        let res = test::call_service(&router.app, req).await;

        assert!(res.status().is_success(), "Expected 200 OK");

        let content_type_header = get_content_type_header(&res);

        let body = test::read_body(res).await;
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
        let _subgraphs_server = SubgraphsServer::start().await;

        let router = init_router_from_config_inline(&format!(
            r#"
            supergraph:
                source: file
                path: supergraph.graphql
            "#
        ))
        .await
        .unwrap();

        wait_for_readiness(&router.app).await;

        let req = init_graphql_request(
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
        )
        .header(http::header::ACCEPT, "text/event-stream")
        .to_request();

        let res = test::call_service(&router.app, req).await;

        assert!(res.status().is_success(), "Expected 200 OK");

        let content_type_header = get_content_type_header(&res);

        let body = test::read_body(res).await;
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
        let subgraphs_server = SubgraphsServer::start().await;

        let router = init_router_from_config_inline(&format!(
            r#"
            supergraph:
                source: file
                path: supergraph.graphql
            "#
        ))
        .await
        .unwrap();

        wait_for_readiness(&router.app).await;

        let req = init_graphql_request(
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
        )
        .header(http::header::ACCEPT, "text/event-stream")
        .to_request();

        let res = test::call_service(&router.app, req).await;

        assert!(res.status().is_success(), "Expected 200 OK");

        let subgraph_request = subgraphs_server
            .get_subgraph_requests_log("reviews")
            .await
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

        assert_snapshot!(accept_header, @r#"multipart/mixed; boundary="graphql"; subscriptionSpec="1.0", text/event-stream"#);
    }

    #[ntex::test]
    async fn subscription_stream_failed_source_subgraph_requests() {
        let _subgraphs_server = SubgraphsServer::start_with_interceptor(Arc::new(|_req| {
            Some(InterceptedResponse::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                None,
            ))
        }))
        .await;

        let router = init_router_from_config_inline(&format!(
            r#"
            supergraph:
                source: file
                path: supergraph.graphql
            "#
        ))
        .await
        .unwrap();

        wait_for_readiness(&router.app).await;

        let req = init_graphql_request(
            r#"
            subscription {
                reviewAdded(intervalInMs: 0) {
                    id
                }
            }
            "#,
            None,
        )
        .header(http::header::ACCEPT, "text/event-stream")
        .to_request();

        let res = test::call_service(&router.app, req).await;

        let body = test::read_body(res).await;
        let body_str = std::str::from_utf8(&body).unwrap();

        assert_snapshot!(body_str, @r#"
        event: next
        data: {"data":null,"errors":[{"message":"Failed to execute request to subgraph","extensions":{"code":"SUBGRAPH_REQUEST_FAILURE","serviceName":"reviews"}}]}

        event: complete
        "#);
    }
}
