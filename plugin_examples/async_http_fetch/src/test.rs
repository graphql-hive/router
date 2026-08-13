#[cfg(test)]
mod tests {
    use e2e::{
        mockito,
        testkit::{TestRouter, TestSubgraphs},
    };
    use hive_router::{ntex, sonic_rs};

    #[ntex::test]
    async fn awaits_upstream_fetch_before_the_request_continues() {
        let mut server = mockito::Server::new_async().await;
        let greeting_mock = server
            .mock("GET", "/greeting")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(sonic_rs::json!({ "greeting": "hello from upstream" }).to_string())
            .expect(1)
            .create_async()
            .await;

        let subgraphs = TestSubgraphs::builder().build().start().await;

        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .skip_wait_for_healthy_on_start()
            .skip_wait_for_ready_on_start()
            .inline_config(format!(
                r#"
                supergraph:
                    source: file
                    path: ../../e2e/supergraph.graphql
                plugins:
                    async_http_fetch:
                        enabled: true
                        config:
                            upstream_url: "http://{}/greeting"
                "#,
                server.host_with_port()
            ))
            .register_plugin::<crate::plugin::AsyncHttpFetchPlugin>()
            .build()
            .start()
            .await;

        let res = router.send_graphql_request("{ me { name } }", None, None).await;

        assert!(res.status().is_success(), "Expected 200 OK");
        assert_eq!(
            res.headers().get("x-fetched-greeting").unwrap(),
            "hello from upstream"
        );

        greeting_mock.assert_async().await;
    }
}
