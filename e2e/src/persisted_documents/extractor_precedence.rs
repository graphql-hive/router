use sonic_rs::json;

use super::shared::{assert_resolves_successfully, write_manifest, DOC_ID};
use crate::testkit::{TestRouter, TestSubgraphs};

#[ntex::test]
// Extractors are applied in order, and the first match wins.
async fn uses_first_match() {
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
                      path: documentId
                    - type: url_query_param
                      name: documentId
                "#,
            manifest.path().display(),
        ))
        .build()
        .start()
        .await;

    let response = router
        .serv()
        .post("/graphql?documentId=notfound")
        .send_json(&json!({ "documentId": DOC_ID }))
        .await
        .expect("failed to send graphql request");

    assert_resolves_successfully(response).await;
}
