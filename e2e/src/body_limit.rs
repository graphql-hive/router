#[cfg(test)]
mod body_limit_e2e_tests {
    use crate::testkit::{init_graphql_request, init_router_from_config_file, wait_for_readiness};
    use ntex::web::test;

    #[ntex::test]
    async fn should_return_payload_too_large_if_limit_exceeds() {
        let app = init_router_from_config_file("configs/body_limit.yaml")
            .await
            .unwrap();
        wait_for_readiness(&app.app).await;

        let req = init_graphql_request("{ __typename }", None);
        let resp = test::call_service(&app.app, req.to_request()).await;
        // Check the status code
        assert_eq!(resp.status(), ntex::http::StatusCode::PAYLOAD_TOO_LARGE);

        let body = test::read_body(resp).await;
        let json_body: sonic_rs::Value =
            sonic_rs::from_slice(&body).expect("The response body should be valid JSON");

        insta::assert_snapshot!(sonic_rs::to_string_pretty(&json_body).expect("The JSON body should be serializable to a pretty string"), @r###"
        {
          "errors": [
            {
              "message": "Request body size exceeds the limit",
              "extensions": {
                "code": "PAYLOAD_TOO_LARGE"
              }
            }
          ]
        }
        "###);
    }
}
