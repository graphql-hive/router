#[cfg(test)]
mod probes_e2e_tests {
    use ntex::web::test;
    use std::{
        thread::{self},
        time::Duration,
    };

    use crate::testkit::{init_router_from_config_inline, wait_for_readiness};

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

        let app = init_router_from_config_inline(&format!(
            r#"supergraph:
              source: hive
              endpoint: http://{host}/supergraph
              key: dummy_key
              poll_interval: 500ms
        "#,
        ))
        .await
        .expect("failed to start router");

        // At the point, if supergraph is not loaded yet, health should be OK 200
        let req = test::TestRequest::get().uri("/health").to_request();
        let response = app.call(req).await.expect("failed to check health");
        assert!(response.status().is_success());

        // And readiness should be 500 with server error
        let req = test::TestRequest::get().uri("/readiness").to_request();
        let response = app.call(req).await.expect("failed to check readiness");
        assert!(response.status().is_server_error());

        wait_for_readiness(&app.app).await;

        // And now when it's finally loaded, both should be 200 OK
        let req = test::TestRequest::get().uri("/health").to_request();
        let response = app.call(req).await.expect("failed to check health");
        assert!(response.status().is_success());

        // And readiness should be 500 with server error
        let req = test::TestRequest::get().uri("/readiness").to_request();
        let response = app.call(req).await.expect("failed to check readiness");
        assert!(response.status().is_success());
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

        let app = init_router_from_config_inline(&format!(
            r#"supergraph:
            source: hive
            endpoint: http://{host}/supergraph
            key: dummy_key
            poll_interval: 500ms
      "#,
        ))
        .await
        .expect("failed to start router");

        wait_for_readiness(&app.app).await;

        // at first, both should be ok
        let req = test::TestRequest::get().uri("/health").to_request();
        let response = app.call(req).await.expect("failed to check health");
        assert!(response.status().is_success());
        let req = test::TestRequest::get().uri("/readiness").to_request();
        let response = app.call(req).await.expect("failed to check readiness");
        assert!(response.status().is_success());

        // Then, we wait for >500ms and check readiness again, it should be valid
        // even if the router fails to load it from the source
        ntex::time::sleep(Duration::from_millis(600)).await;
        let req = test::TestRequest::get().uri("/readiness").to_request();
        let response = app.call(req).await.expect("failed to check readiness");
        assert!(response.status().is_success());

        // we give it some more time, to deal with retries, but the idea is the same
        ntex::time::sleep(Duration::from_millis(1000)).await;
        let req = test::TestRequest::get().uri("/readiness").to_request();
        let response = app.call(req).await.expect("failed to check readiness");
        assert!(response.status().is_success());
    }
}
