use sonic_rs::json;

use super::shared::{assert_error_code, write_manifest};
use crate::testkit::{TestRouter, TestSubgraphs};

#[ntex::test]
// Empty documentId is treated as missing
async fn empty_id_is_ignored() {
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
                  extractors:
                    - type: json_path
                      path: documentId
                "#,
            manifest.path().display(),
        ))
        .build()
        .start()
        .await;

    let response = router
        .serv()
        .post("/graphql")
        .send_json(&json!({ "documentId": "" }))
        .await
        .expect("failed to send graphql request");

    assert_error_code(response, "PERSISTED_DOCUMENT_ID_REQUIRED").await;
}

#[ntex::test]
// Make sure non-string values are accepted by the document id extractor
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
            "documentId": 123
        }))
        .await
        .expect("failed to send graphql request");

    // If Hive Router does not support u64,
    // the error code would be PERSISTED_DOCUMENT_ID_REQUIRED
    assert_error_code(response, "PERSISTED_DOCUMENT_NOT_FOUND").await;
}
