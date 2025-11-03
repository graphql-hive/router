#[cfg(test)]
mod hmac_e2e_tests {
    use crate::testkit::{
        init_graphql_request, init_router_from_config_file, wait_for_readiness, SubgraphsServer,
    };

    use ntex::web::test;
    use sonic_rs::JsonValueTrait;

    fn create_hmac_signature(secret: &str, query: &str) -> String {
        use hex;
        use hmac::{Hmac, Mac};
        use sha2::Sha256;

        type HmacSha256 = Hmac<Sha256>;

        let mut mac =
            HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC can take key of any size");
        let message = format!("{{\"query\":\"{}\"}}", query);
        mac.update(message.as_bytes());
        let result = mac.finalize();
        let code_bytes = result.into_bytes();
        hex::encode(code_bytes)
    }

    #[ntex::test]
    async fn should_forward_hmac_signature_to_subgraph_via_extensions() {
        let subgraphs_server = SubgraphsServer::start().await;
        let app = init_router_from_config_file("configs/hmac_forward.router.yaml")
            .await
            .unwrap();
        wait_for_readiness(&app.app).await;
        let query = "query{users{id}}";
        let req = init_graphql_request(query, None);
        let resp: ntex::web::WebResponse = test::call_service(&app.app, req.to_request()).await;

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
        let extensions = subgraph_requests[0].request_body.get("extensions").unwrap();

        let expected_signature = create_hmac_signature("VERY_SECRET", query);
        assert_eq!(
            extensions.get("hmac_signature").unwrap(),
            &expected_signature
        );
    }
}
