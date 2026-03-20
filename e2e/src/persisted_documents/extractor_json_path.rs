use sonic_rs::json;

use super::shared::{assert_error_code, assert_resolves_successfully, write_manifest, DOC_ID};
use crate::testkit::{TestRouter, TestSubgraphs};

#[ntex::test]
// Make sure json_path extractor extracts from extensions nested path
async fn extracts_from_extensions_nested_path() {
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
                      path: extensions.custom.document.id
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
                "custom": {
                    "document": {
                        "id": DOC_ID
                    }
                }
            }
        }))
        .await
        .expect("failed to send graphql request");

    assert_resolves_successfully(response).await;
}

#[ntex::test]
// Make sure json_path extractor extracts from non-standard root field
async fn extracts_from_nonstandard_root_field() {
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
                      path: custom.document.id
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
            "custom": {
                "document": {
                    "id": DOC_ID
                }
            }
        }))
        .await
        .expect("failed to send graphql request");

    assert_resolves_successfully(response).await;
}

#[ntex::test]
// Make sure json_path extractor does not error when path is missing,
// but returns none instead
async fn returns_none_when_path_missing() {
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
                      path: extensions.custom.document.id
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
                "custom": {}
            }
        }))
        .await
        .expect("failed to send graphql request");

    assert_error_code(response, "PERSISTED_DOCUMENT_ID_REQUIRED").await;
}
