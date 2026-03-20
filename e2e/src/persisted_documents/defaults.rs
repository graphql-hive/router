use sonic_rs::json;

use super::shared::{assert_resolves_successfully, write_manifest, DOC_ID};
use crate::testkit::{TestRouter, TestSubgraphs};

#[ntex::test]
// Make sure Hive Router accepts by default:
// - json: documentId
// - json: extensions.persistedQuery.sha256
async fn default_extractors() {
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

    assert_resolves_successfully(response).await;

    let response = router
        .send_post_request(
            "/graphql",
            json!({
                "extensions": {
                    "persistedQuery": {
                        "sha256Hash": DOC_ID
                    }
                }
            }),
            None,
        )
        .await;

    assert_resolves_successfully(response).await;
}
