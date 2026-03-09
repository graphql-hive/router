#[cfg(test)]
mod http_callback_e2e_tests {
    use insta::assert_snapshot;
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
                        http::header::ACCEPT => "text/event-stream"
                ),
            )
            .await;

        assert_eq!(res.status(), 200, "Expected 200 OK");

        assert_snapshot!(res.string_body().await, @r#"
        event: next
        data: {"data":{"reviewAdded":{"id":"1","product":{"name":"Table"}}}}

        event: next
        data: {"data":{"reviewAdded":{"id":"2","product":{"name":"Table"}}}}

        event: next
        data: {"data":{"reviewAdded":{"id":"3","product":{"name":"Table"}}}}

        event: next
        data: {"data":{"reviewAdded":{"id":"4","product":{"name":"Table"}}}}

        event: next
        data: {"data":null,"errors":[{"message":"Subgraph gone due heartbeat timeout","extensions":{"code":"SUBGRAPH_GONE"}}]}

        event: complete
        "#);
    }
}
