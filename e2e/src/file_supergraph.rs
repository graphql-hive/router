#[cfg(test)]
mod file_supergraph_e2e_tests {
    use sonic_rs::{from_slice, JsonContainerTrait, JsonValueTrait, Value};
    use std::{fs, time::Duration};
    use tempfile::NamedTempFile;

    use crate::testkit::TestRouterBuilder;

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

        let router = TestRouterBuilder::new()
            .inline_config(format!(
                r#"
                supergraph:
                    source: file
                    path: {supergraph_file_path}
                "#,
            ))
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request("{ __schema { types { name } } }", None, None)
            .await;

        assert!(res.status().is_success(), "Expected 200 OK");

        let body = res.body().await.unwrap();
        let json_body: Value = from_slice(&body).unwrap();
        let types_arr = json_body
            .get("data")
            .unwrap()
            .get("__schema")
            .unwrap()
            .get("types")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(types_arr.len(), 18);
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

        let router = TestRouterBuilder::new()
            .inline_config(format!(
                r#"
                supergraph:
                    source: file
                    path: {supergraph_file_path}
                    poll_interval: 100ms
                "#,
            ))
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request("{ __schema { types { name } } }", None, None)
            .await;

        assert!(res.status().is_success(), "Expected 200 OK");

        let body = res.body().await.unwrap();
        let json_body: Value = from_slice(&body).unwrap();
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
            let res = router
                .send_graphql_request("{ __schema { types { name } } }", None, None)
                .await;

            if res.status().is_success() {
                let body = res.body().await.unwrap();
                if String::from_utf8_lossy(&body).contains("NewType") {
                    break;
                }
            }

            attempts += 1;
            if attempts >= max_attempts {
                panic!("Supergraph did not reload within timeout");
            }

            tokio::time::sleep(Duration::from_millis(interval_ms)).await;
        }

        let res = router
            .send_graphql_request("{ __schema { types { name } } }", None, None)
            .await;

        assert!(res.status().is_success(), "Expected 200 OK");

        let body = res.body().await.unwrap();
        let json_body: Value = from_slice(&body).unwrap();
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
