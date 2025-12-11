#[cfg(test)]

mod header_propagation_e2e_tests {
    use ntex::web::test;

    use crate::testkit::{
        init_graphql_request, init_router_from_config_file, wait_for_readiness, SubgraphsServer,
    };

    #[ntex::test]
    async fn should_propagate_headers_to_subgraphs() {
        let subgraphs_server = SubgraphsServer::start().await;
        let app = init_router_from_config_file("configs/header_propagation.router.yaml")
            .await
            .unwrap();
        wait_for_readiness(&app.app).await;
        let req =
            init_graphql_request("{ users { id } }", None).header("x-context", "my-context-value");
        let resp = test::call_service(&app.app, req.to_request()).await;

        assert!(resp.status().is_success(), "Expected 200 OK");

        let subgraph_requests = subgraphs_server
            .get_subgraph_requests_log("accounts")
            .await
            .expect("expected requests sent to accounts subgraph");
        assert_eq!(
            subgraph_requests.len(),
            1,
            "expected 1 request to accounts subgraph"
        );

        let last_request = &subgraph_requests[0];
        let headers = &last_request.headers;
        let context_header = headers
            .get("x-context")
            .expect("expected x-context header to be present");
        assert_eq!(
            context_header, "my-context-value",
            "expected x-context header to be propagated to subgraph"
        );
    }
}
