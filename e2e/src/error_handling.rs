#[cfg(test)]
mod error_handling_e2e_tests {
    use ntex::web::test;
    use sonic_rs::{from_slice, JsonContainerTrait, JsonValueTrait, Value};

    use crate::testkit::{
        init_graphql_request, init_router_from_config_file, wait_for_readiness, SubgraphsServer,
    };

    #[ntex::test]
    async fn should_continue_execution_when_a_subgraph_is_down() {
        let subgraphs_server = SubgraphsServer::start_with_port(4100).await;

        let app = init_router_from_config_file("configs/error_handling.router.yaml")
            .await
            .unwrap();
        wait_for_readiness(&app.app).await;

        let req = init_graphql_request("{ me { reviews { id product { upc name } } } }", None);
        let resp = test::call_service(&app.app, req.to_request()).await;

        assert!(resp.status().is_success(), "Expected 200 OK");
        let resp_body_bytes = test::read_body(resp).await;
        let resp_json: Value = from_slice(&resp_body_bytes).expect("expected valid JSON response");

        let data = resp_json
            .get("data")
            .expect("expected 'data' field in response");
        let me = data
            .get("me")
            .expect("expected 'me' field in response data");
        let reviews = me
            .get("reviews")
            .expect("expected 'reviews' field in 'me' object");
        let reviews = reviews
            .as_array()
            .expect("expected 'reviews' field to be an array");
        assert_eq!(reviews.len(), 2, "expected 2 reviews in response");
        let first_review = &reviews[0];
        let first_review_id = first_review
            .get("id")
            .expect("expected 'id' field in first review");
        assert_eq!(
            first_review_id, "1",
            "expected first review id to be 'review-1'"
        );
        // Product can be present because we know `upc`
        let first_review_product = first_review
            .get("product")
            .expect("expected 'product' field in first review");
        // Upc is there
        let first_review_product_upc = first_review_product
            .get("upc")
            .expect("expected 'upc' field in first review's product");
        assert_eq!(
            first_review_product_upc, "1",
            "expected first review's product upc to be '1'"
        );
        // Name is null because products subgraph is down
        let first_review_product_name = first_review_product
            .get("name")
            .expect("expected 'name' field in first review's product");
        assert!(
            first_review_product_name.is_null(),
            "expected first review's product name to be null because products subgraph is down"
        );
        // Check if error is present
        let errors = resp_json
            .get("errors")
            .expect("expected 'errors' field in response");
        let errors = errors
            .as_array()
            .expect("expected 'errors' field to be an array");
        assert_eq!(errors.len(), 1, "expected 1 error in response");
        let first_error = &errors[0];
        // Check if serviceName is present in extensions
        let extensions = first_error
            .get("extensions")
            .expect("expected 'extensions' field in first error");
        let service_name = extensions
            .get("serviceName")
            .expect("expected 'serviceName' field in error extensions");
        assert_eq!(
            service_name, "products",
            "expected serviceName to be 'products'"
        );

        assert_eq!(
            subgraphs_server
                .get_subgraph_requests_log("accounts")
                .await
                .expect("expected requests sent to accounts subgraph")
                .len(),
            1,
            "expected 1 request to accounts subgraph"
        );
    }
}
