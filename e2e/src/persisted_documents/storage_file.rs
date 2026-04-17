use std::time::Duration;

use sonic_rs::json;

use super::shared::{assert_error_code, assert_resolves_successfully, write_manifest, DOC_ID};
use crate::testkit::{TestRouter, TestSubgraphs};

#[ntex::test]
// Make sure the file watch works as expected and updates the manifest.
async fn file_watch_works() {
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
        .send_post_request("/graphql", json!({ "documentId": DOC_ID }), None)
        .await;

    // We expect the first request to resolve successfully,
    // because the DOC_ID is present in the manifest.
    assert_resolves_successfully(response).await;

    // Now we replace the manifest with new content,
    // that lacks the DOC_ID.
    std::fs::write(manifest.path(), r#"{"foo":"{__typename}"}"#)
        .expect("failed to update manifest");

    // Debounce of 150ms is configured for file watch events,
    // so let's wait a double the time before making the request.
    tokio::time::sleep(Duration::from_millis(300)).await;

    let response = router
        .send_post_request(
            "/graphql",
            json!({
                "documentId": DOC_ID
            }),
            None,
        )
        .await;

    // We expect the request to fail,
    // because the DOC_ID is no longer present in the manifest.
    assert_error_code(response, "PERSISTED_DOCUMENT_NOT_FOUND").await;
}
