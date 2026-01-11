#[cfg(test)]
mod hive_cdn_supergraph_e2e_tests {
    use ntex::web::test;
    use sonic_rs::json;
    use std::fs;
    use tempfile::NamedTempFile;

    use crate::testkit::{init_router_from_config_inline, wait_for_readiness};

    #[ntex::test]
    async fn should_allow_to_customize_graphql_endpoint() {
        let file = NamedTempFile::new().expect("failed to create temp file");
        let supergraph_file_path = file
            .path()
            .to_str()
            .expect("failed to convert path to string")
            .to_string();

        let first_supergraph = include_str!("../supergraph.graphql");
        fs::write(&supergraph_file_path, first_supergraph).expect("failed to write supergraph");

        let app = init_router_from_config_inline(&format!(
            r#"
            supergraph:
              source: file
              path: {supergraph_file_path}
            http:
              graphql_endpoint: /custom
        "#,
        ))
        .await
        .expect("failed to start router");
        wait_for_readiness(&app.app).await;

        let body = json!({
          "query": "{ __schema { types { name } } }",
        });

        let req = test::TestRequest::post()
            .uri("/custom")
            .header("content-type", "application/json")
            .set_payload(body.to_string());

        let resp = test::call_service(&app.app, req.to_request()).await;

        assert!(resp.status().is_success(), "Expected 200 OK");

        let req = test::TestRequest::post()
            .uri("/graphql")
            .header("content-type", "application/json")
            .set_payload(body.to_string());

        let resp = test::call_service(&app.app, req.to_request()).await;

        assert_eq!(resp.status(), 404);
    }
}
