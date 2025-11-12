#[cfg(test)]
mod persisted_documents_e2e_tests {
    use ntex::web::test;
    use sonic_rs::json;

    use crate::testkit::{init_router_from_config_file, wait_for_readiness, SubgraphsServer};

    #[ntex::test]
    /// Tests a simple persisted document from a file retrieval using the "hive" spec.
    async fn should_get_persisted_document_from_a_file() {
        let subgraphs_server = SubgraphsServer::start().await;
        let app = init_router_from_config_file("configs/persisted_documents/file_source.yaml")
            .await
            .unwrap();
        wait_for_readiness(&app.app).await;

        let body = json!({
           "documentId": "simple",
        });

        let req = test::TestRequest::post()
            .uri("/graphql")
            .header("content-type", "application/json")
            .set_payload(body.to_string());
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
    }

    #[ntex::test]
    /// Tests a persisted document retrieval using a custom expression spec.
    async fn should_support_custom_spec() {
        let subgraphs_server = SubgraphsServer::start().await;
        let app = init_router_from_config_file("configs/persisted_documents/expr_spec.yaml")
            .await
            .unwrap();
        wait_for_readiness(&app.app).await;

        let body = json!({
            "my_id": "simple",
        });

        let req = test::TestRequest::post()
            .uri("/graphql")
            .header("content-type", "application/json")
            .set_payload(body.to_string());
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
    }

    #[ntex::test]
    /// Tests if arbitrary operations are allowed based on a custom expression.
    async fn should_allow_arbitrary_operations_based_on_expression() {
        let subgraphs_server = SubgraphsServer::start().await;
        let app =
            init_router_from_config_file("configs/persisted_documents/expr_allow_arbitrary.yaml")
                .await
                .unwrap();
        wait_for_readiness(&app.app).await;

        let arbitrary = json!({
            "query": "{ users { id } }",
        });

        let req = test::TestRequest::post()
            .uri("/graphql")
            .header("content-type", "application/json")
            .header("x-allow-arbitrary-operations", "true")
            .set_payload(arbitrary.to_string());
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

        let not_allowed = json!({
            "query": "{ users { id } }",
        });

        let req = test::TestRequest::post()
            .uri("/graphql")
            .header("content-type", "application/json")
            .set_payload(not_allowed.to_string());
        let resp = test::call_service(&app.app, req.to_request()).await;
        assert_eq!(resp.status().as_u16(), 400, "Expected 400 Bad Request");
    }

    #[ntex::test]
    /// Tests if arbitrary operations are allowed based on the Apollo spec.
    async fn should_support_apollo_spec() {
        let subgraphs_server = SubgraphsServer::start().await;
        let app = init_router_from_config_file("configs/persisted_documents/apollo_spec.yaml")
            .await
            .unwrap();
        wait_for_readiness(&app.app).await;

        let body = json!({
            "extensions": {
                "persistedQuery": {
                    "version": 1,
                    "sha256Hash": "simple"
                }
            }
        });

        let req = test::TestRequest::post()
            .uri("/graphql")
            .header("content-type", "application/json")
            .set_payload(body.to_string());

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
    }
    #[ntex::test]
    async fn should_support_url_params() {
        let subgraphs_server = SubgraphsServer::start().await;
        let app = init_router_from_config_file("configs/persisted_documents/url_spec.yaml")
            .await
            .unwrap();
        wait_for_readiness(&app.app).await;

        let req = test::TestRequest::get().uri("/graphql/simple");
        let resp = test::call_service(&app.app, req.to_request()).await;

        let status = resp.status();
        let body = test::read_body(resp).await;
        assert!(
            status.is_success(),
            "Expected 200 OK, got {} with body {:#?}",
            status,
            body
        );

        let subgraph_requests = subgraphs_server
            .get_subgraph_requests_log("accounts")
            .await
            .expect("expected requests sent to accounts subgraph");
        assert_eq!(
            subgraph_requests.len(),
            1,
            "expected 1 request to accounts subgraph"
        );
    }
}
