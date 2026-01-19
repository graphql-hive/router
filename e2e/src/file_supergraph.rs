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
        ))
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
        ))
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
        let types_arr: Vec<String> = json_body
            .get("data")
            .unwrap()
            .get("__schema")
            .unwrap()
            .get("types")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(|i| {
                i.as_object()
                    .unwrap()
                    .get(&"name")
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect();

        assert_eq!(
            types_arr.contains(&"Query".to_string()),
            true,
            "Expected types to contain 'Query'"
        );
        assert_eq!(
            types_arr.contains(&"NewType".to_string()),
            false,
            "Expected types to not contain 'NewType'"
        );

        fs::write(
            &supergraph_file_path,
            "type Query { dummyNew: NewType } type NewType { id: ID! }",
        )
        .expect("failed to write supergraph");

        // Poll for the supergraph to be reloaded
        let interval_ms = 50;
        let mut attempts = 0;
        let max_attempts = 10; // 10 * 50ms = 500 ms max wait
        loop {
            let resp = test::call_service(
                &app.app,
                init_graphql_request("{ __schema { types { name } } }", None).to_request(),
            )
            .await;

            if resp.status().is_success() {
                if String::from_utf8_lossy(&test::read_body(resp).await).contains("NewType") {
                    break;
                }
            }

            attempts += 1;
            if attempts >= max_attempts {
                panic!("Supergraph did not reload within timeout");
            }

            time::sleep(Duration::from_millis(interval_ms)).await;
        }

        let resp = test::call_service(
            &app.app,
            init_graphql_request("{ __schema { types { name } } }", None).to_request(),
        )
        .await;

        assert!(resp.status().is_success(), "Expected 200 OK");

        let json_body: Value = from_slice(&test::read_body(resp).await).unwrap();
        let types_arr: Vec<String> = json_body
            .get("data")
            .unwrap()
            .get("__schema")
            .unwrap()
            .get("types")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(|i| {
                i.as_object()
                    .unwrap()
                    .get(&"name")
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect();
        assert_eq!(
            types_arr.contains(&"Query".to_string()),
            true,
            "Expected types to contain 'Query'"
        );
        assert_eq!(
            types_arr.contains(&"NewType".to_string()),
            true,
            "Expected types to contain 'NewType'"
        );
    }
}
