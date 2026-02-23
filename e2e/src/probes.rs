#[cfg(test)]
mod probes_e2e_tests {
    use std::{
        thread::{self},
        time::Duration,
    };

    use crate::testkit_v2::TestRouterBuilder;

    #[ntex::test]
    async fn should_respond_to_probes_correctly() {
        let mut server = mockito::Server::new_async().await;
        let host = server.host_with_port();

        server
            .mock("GET", "/supergraph")
            .with_status(200)
            .with_header("content-type", "text/plain")
            .with_chunked_body(|w| {
                thread::sleep(Duration::from_secs(1));
                w.write_all(include_str!("../supergraph.graphql").as_bytes())
            })
            .create();

        let router = TestRouterBuilder::new()
            .inline_config(&format!(
                r#"
                supergraph:
                    source: hive
                    endpoint: http://{host}/supergraph
                    key: dummy_key
                    poll_interval: 500ms
                "#,
            ))
            .skip_wait_for_healthy_on_start()
            .skip_wait_for_ready_on_start()
            .build()
            .start()
            .await;

        // At the point, if supergraph is not loaded yet, health should be OK 200
        let res = router.serv().post("/health").send().await.unwrap();
        assert!(res.status().is_success());

        // And readiness should be 500 with server error
        let res = router.serv().post("/readiness").send().await.unwrap();
        assert!(res.status().is_server_error());

        router.wait_for_ready(None).await;

        // And now when it's finally loaded, both should be 200 OK
        let res = router.serv().post("/health").send().await.unwrap();
        assert!(res.status().is_success());

        // And readiness should be 200 too
        let res = router.serv().post("/readiness").send().await.unwrap();
        assert!(res.status().is_success());
    }

    #[ntex::test]
    async fn should_not_fail_readiness_when_supergraph_fails_to_reload() {
        let mut server = mockito::Server::new_async().await;
        let host = server.host_with_port();

        server
            .mock("GET", "/supergraph")
            .expect(1)
            .with_status(200)
            .with_header("content-type", "text/plain")
            .with_body(include_str!("../supergraph.graphql"))
            .create();

        server
            .mock("GET", "/supergraph")
            .expect_at_least(1)
            .with_status(500)
            .create();

        let router = TestRouterBuilder::new()
            .inline_config(&format!(
                r#"
                supergraph:
                    source: hive
                    endpoint: http://{host}/supergraph
                    key: dummy_key
                    poll_interval: 500ms
                "#,
            ))
            // wait for both in this test
            // .skip_wait_for_healthy_on_start()
            // .skip_wait_for_ready_on_start()
            .build()
            .start()
            .await;

        // at first, both should be ok
        let res = router.serv().post("/health").send().await.unwrap();
        assert!(res.status().is_success());
        let res = router.serv().post("/readiness").send().await.unwrap();
        assert!(res.status().is_success());

        // Then, we wait for >500ms and check readiness again, it should be valid
        // even if the router fails to load it from the source
        ntex::time::sleep(Duration::from_millis(600)).await;

        let res = router.serv().post("/readiness").send().await.unwrap();
        assert!(res.status().is_success());

        // we give it some more time, to deal with retries, but the idea is the same
        ntex::time::sleep(Duration::from_millis(1000)).await;
        let res = router.serv().post("/readiness").send().await.unwrap();
        assert!(res.status().is_success());
    }
}
