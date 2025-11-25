#[cfg(test)]
mod file_supergraph_e2e_tests {
    use ntex::{time, web::test};
    use sonic_rs::{from_slice, JsonContainerTrait, JsonValueTrait, Value};
    use std::{fs, time::Duration};
    use tempfile::NamedTempFile;

    use crate::testkit::{
        init_graphql_request, init_router_from_config_inline, wait_for_readiness,
    };

    #[ntex::test]
    async fn should_load_supergraph_from_file() {
        let file = NamedTempFile::new().expect("failed to create temp file");
        let supergraph_file_path = file
            .path()
            .to_str()
            .expect("failed to convert path to string")
            .to_string();

        let first_supergraph = include_str!("../supergraph.graphql");
        fs::write(&supergraph_file_path, first_supergraph).expect("failed to write supergraph");

        let app = init_router_from_config_inline(&format!(
            r#"supergraph:
                source: file
                path: {supergraph_file_path}
          "#,
        ), None)
        .await
        .expect("failed to start router");
        wait_for_readiness(&app.app).await;

        let resp = test::call_service(
            &app.app,
            init_graphql_request("{ __schema { types { name } } }", None).to_request(),
        )
        .await;

        assert!(resp.status().is_success(), "Expected 200 OK");

        let json_body: Value = from_slice(&test::read_body(resp).await).unwrap();
        let types_arr = json_body
            .get("data")
            .unwrap()
            .get("__schema")
            .unwrap()
            .get("types")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(types_arr.len(), 17);
    }

    #[ntex::test]
    async fn should_reload_supergraph_from_file() {
        let file = NamedTempFile::new().expect("failed to create temp file");
        let supergraph_file_path = file
            .path()
            .to_str()
            .expect("failed to convert path to string")
            .to_string();

        fs::write(&supergraph_file_path, "type Query { f: String }")
            .expect("failed to write supergraph");

        let app = init_router_from_config_inline(&format!(
            r#"supergraph:
                source: file
                path: {supergraph_file_path}
                poll_interval: 100ms
          "#,
        ), None)
        .await
        .expect("failed to start router");
        wait_for_readiness(&app.app).await;

        let resp = test::call_service(
            &app.app,
            init_graphql_request("{ __schema { types { name } } }", None).to_request(),
        )
        .await;

        assert!(resp.status().is_success(), "Expected 200 OK");

        let json_body: Value = from_slice(&test::read_body(resp).await).unwrap();
        let types_arr = json_body
            .get("data")
            .unwrap()
            .get("__schema")
            .unwrap()
            .get("types")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(types_arr.len(), 14);

        fs::write(
            &supergraph_file_path,
            "type Query { dummyNew: NewType } type NewType { id: ID! }",
        )
        .expect("failed to write supergraph");
        time::sleep(Duration::from_millis(150)).await;

        let resp = test::call_service(
            &app.app,
            init_graphql_request("{ __schema { types { name } } }", None).to_request(),
        )
        .await;

        assert!(resp.status().is_success(), "Expected 200 OK");

        let json_body: Value = from_slice(&test::read_body(resp).await).unwrap();
        let types_arr = json_body
            .get("data")
            .unwrap()
            .get("__schema")
            .unwrap()
            .get("types")
            .unwrap()
            .as_array()
            .unwrap();
        // one more type added
        assert_eq!(types_arr.len(), 15);
    }
}
