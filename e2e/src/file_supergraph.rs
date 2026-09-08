#[cfg(test)]
mod file_supergraph_e2e_tests {
    use sonic_rs::{JsonContainerTrait, JsonValueTrait};
    use std::{fs, time::Duration};
    use tempfile::NamedTempFile;

    use crate::testkit::{ClientResponseExt, TestRouter};

    /// The router is pointed at a symlink and must reload when the symlink is repointed to
    /// a new file and the previous target is deleted. This is the essential mechanic behind
    /// a Kubernetes `ConfigMap` update (kubelet swaps its internal `..data` symlink and
    /// removes the old data): the reader must keep following the symlink rather than pinning
    /// to (and then failing on) the resolved, now-deleted target.
    #[cfg(unix)]
    #[ntex::test]
    async fn should_reload_supergraph_when_symlink_target_swaps() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let link_path = dir.path().join("supergraph.graphql");

        // Atomically repoint `link_path` at a fresh file containing `content`, then remove
        // the file it previously pointed at.
        let swap = |version: u32, content: &str| {
            let target = dir.path().join(format!("supergraph.v{version}.graphql"));
            fs::write(&target, content).expect("failed to write target file");

            let old_target = fs::read_link(&link_path).ok();
            let tmp = dir.path().join(".supergraph.tmp");
            let _ = fs::remove_file(&tmp);
            std::os::unix::fs::symlink(&target, &tmp).expect("failed to create tmp symlink");
            // rename(2) over the existing symlink swaps it atomically.
            fs::rename(&tmp, &link_path).expect("failed to swap symlink");

            if let Some(old) = old_target {
                let _ = fs::remove_file(old);
            }
        };

        swap(1, "type Query { f: String }");
        let supergraph_file_path = link_path.to_str().expect("path is valid utf-8").to_string();

        let router = TestRouter::builder()
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

        // Initial schema: Query present, NewType absent.
        let res = router
            .send_graphql_request("{ __schema { types { name } } }", None, None)
            .await;
        assert!(res.status().is_success(), "Expected 200 OK");
        let body = res.body().await.unwrap();
        assert!(
            !String::from_utf8_lossy(&body).contains("NewType"),
            "Expected initial schema to not contain 'NewType'"
        );

        // Poll the router until the schema reloaded to contain `expected_type`.
        let router_ref = &router;
        let wait_for_type = move |expected_type: &'static str| async move {
            for _ in 0..20 {
                let res = router_ref
                    .send_graphql_request("{ __schema { types { name } } }", None, None)
                    .await;
                if res.status().is_success() {
                    let body = res.body().await.unwrap();
                    if String::from_utf8_lossy(&body).contains(expected_type) {
                        return true;
                    }
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            false
        };

        // Swap the symlink target (and delete the old file). The router must pick up the
        // new schema through the stable symlink path.
        swap(
            2,
            "type Query { dummyNew: NewType } type NewType { id: ID! }",
        );
        assert!(
            wait_for_type("NewType").await,
            "Supergraph did not reload after the symlink target was swapped"
        );

        // A second swap must reload again, proving the reader keeps following the symlink
        // rather than caching a resolved (now-deleted) path.
        swap(
            3,
            "type Query { second: SecondType } type SecondType { id: ID! }",
        );
        assert!(
            wait_for_type("SecondType").await,
            "Supergraph did not reload after a second symlink target swap"
        );
    }

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

        let router = TestRouter::builder()
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

        let json_body = res.json_body().await;
        let types_arr = json_body
            .get("data")
            .unwrap()
            .get("__schema")
            .unwrap()
            .get("types")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(types_arr.len(), 23);
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

        let router = TestRouter::builder()
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

        let json_body = res.json_body().await;
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

        let json_body = res.json_body().await;
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
