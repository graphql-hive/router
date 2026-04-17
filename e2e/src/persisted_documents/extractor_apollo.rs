use sonic_rs::json;

use super::shared::{assert_error_code, assert_resolves_successfully, write_manifest, DOC_ID};
use crate::testkit::{TestRouter, TestSubgraphs};

#[ntex::test]
// Make sure apollo's PQ format works
async fn extracts_sha256_hash_from_extensions() {
    let manifest = write_manifest();
    let subgraphs = TestSubgraphs::builder().build().start().await;
    let router = TestRouter::builder()
        .with_subgraphs(&subgraphs)
        .inline_config(format!(
            r#"
                supergraph:
                  source: file
                  path: supergraph.graphql
                persisted_documents:
                  enabled: true
                  require_id: true
                  storage:
                    type: file
                    path: "{}"
                  selectors:
                    - type: json_path
                      path: extensions.persistedQuery.sha256Hash
                "#,
            manifest.path().display(),
        ))
        .build()
        .start()
        .await;

    let response = router
        .serv()
        .post("/graphql")
        .send_json(&json!({
            "extensions": {
                "persistedQuery": {
                    "sha256Hash": DOC_ID
                }
            }
        }))
        .await
        .expect("failed to send graphql request");

    assert_resolves_successfully(response).await;
}

#[ntex::test]
// Make sure documentId does not collide with apollo's hash
async fn returns_none_when_hash_missing() {
    let manifest = write_manifest();
    let subgraphs = TestSubgraphs::builder().build().start().await;
    let router = TestRouter::builder()
        .with_subgraphs(&subgraphs)
        .inline_config(format!(
            r#"
                supergraph:
                  source: file
                  path: supergraph.graphql
                persisted_documents:
                  enabled: true
                  require_id: true
                  storage:
                    type: file
                    path: "{}"
                  selectors:
                    - type: json_path
                      path: extensions.persistedQuery.sha256Hash
                "#,
            manifest.path().display(),
        ))
        .build()
        .start()
        .await;

    let response = router
        .serv()
        .post("/graphql")
        .send_json(&json!({
            "documentId": "1ab2",
            "extensions": {
                "persistedQuery": {}
            }
        }))
        .await
        .expect("failed to send graphql request");

    assert_error_code(response, "PERSISTED_DOCUMENT_ID_REQUIRED").await;
}

#[ntex::test]
// Make sure non-string values are accepted by the apollo extractor
async fn accepts_non_string_value() {
    let manifest = write_manifest();
    let subgraphs = TestSubgraphs::builder().build().start().await;
    let router = TestRouter::builder()
        .with_subgraphs(&subgraphs)
        .inline_config(format!(
            r#"
                supergraph:
                  source: file
                  path: supergraph.graphql
                persisted_documents:
                  enabled: true
                  require_id: true
                  storage:
                    type: file
                    path: "{}"
                "#,
            manifest.path().display(),
        ))
        .build()
        .start()
        .await;

    let response = router
        .serv()
        .post("/graphql")
        .send_json(&json!({
            "extensions": {
                "persistedQuery": {
                    "sha256Hash": 123
                }
            }
        }))
        .await
        .expect("failed to send graphql request");

    // If Hive Router does not support u64,
    // the error code would be PERSISTED_DOCUMENT_ID_REQUIRED
    assert_error_code(response, "PERSISTED_DOCUMENT_NOT_FOUND").await;
}
