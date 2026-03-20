use sonic_rs::json;

use super::shared::{assert_resolves_successfully, write_manifest};
use crate::testkit::{TestRouter, TestSubgraphs};

#[ntex::test]
async fn query_wins_when_require_id_is_false() {
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
                  require_id: false
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
        .send_post_request(
            "/graphql",
            json!({
                "query": "{ topProducts { name } }",
                "documentId": "sha256:notfound"
            }),
            None,
        )
        .await;

    assert_resolves_successfully(response).await;
}

#[ntex::test]
async fn disabled_mode_ignores_extracted_id() {
    let subgraphs = TestSubgraphs::builder().build().start().await;
    let router = TestRouter::builder()
        .with_subgraphs(&subgraphs)
        .inline_config(
            r#"
                supergraph:
                  source: file
                  path: supergraph.graphql
                persisted_documents:
                  enabled: false
                  require_id: true
                "#,
        )
        .build()
        .start()
        .await;

    let response = router
        .send_post_request(
            "/graphql",
            json!({
                "query": "{ topProducts { name } }",
                "documentId": "sha256:notfound"
            }),
            None,
        )
        .await;

    assert_resolves_successfully(response).await;
}
