#[cfg(test)]
mod http_callback_e2e_tests {
    use ntex::http;

    use crate::testkit::{
        get_available_port, some_header_map, ClientResponseExt, TestRouter, TestSubgraphs,
    };

    #[ntex::test]
    async fn complete_active_subscription_on_heartbeat_timeout() {
        let subgraphs = TestSubgraphs::builder().build().start().await;

        let router_port = get_available_port();
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .with_port(router_port)
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
                        heartbeat_interval: 200ms
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
                        intervalInMs: 100
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
        assert!(body
            .contains(r#"data: {"data":{"reviewAdded":{"id":"1","product":{"name":"Table"}}}}"#));

        // kicked off client
        assert!(body.contains(r#"data: {"data":null,"errors":[{"message":"Failed to execute request to subgraph","extensions":{"code":"SUBGRAPH_SUBSCRIPTION_STREAM_ERROR","serviceName":"reviews"}}]}"#));

        // completed stream
        assert!(body.contains("event: complete"));
    }
}
