use sonic_rs::json;

use super::shared::{assert_error_code, assert_resolves_successfully, write_manifest, PATH_DOC_ID};
use crate::testkit::{TestRouter, TestSubgraphs};

#[ntex::test]
async fn extracts_id_from_path() {
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
                    - type: url_path_param
                      template: /docs/:id
                "#,
            manifest.path().display(),
        ))
        .build()
        .start()
        .await;

    let response = router
        .send_post_request(&format!("/graphql/docs/{PATH_DOC_ID}"), json!({}), None)
        .await;

    assert_resolves_successfully(response).await;
}

#[ntex::test]
async fn mismatch() {
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
                    - type: url_path_param
                      template: /docs/:id
                "#,
            manifest.path().display(),
        ))
        .build()
        .start()
        .await;

    let response = router
        .send_post_request("/graphql/other/abc-123", json!({}), None)
        .await;

    assert_error_code(response, "PERSISTED_DOCUMENT_ID_REQUIRED").await;
}

#[ntex::test]
async fn matches_wildcard_template() {
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
                    - type: url_path_param
                      template: /v1/*/:id/details
                "#,
            manifest.path().display(),
        ))
        .build()
        .start()
        .await;

    let response = router
        .send_post_request(
            &format!("/graphql/v1/anything/{PATH_DOC_ID}/details"),
            json!({}),
            None,
        )
        .await;

    assert_resolves_successfully(response).await;
}

#[ntex::test]
async fn works_with_custom_graphql_endpoint() {
    let manifest = write_manifest();
    let subgraphs = TestSubgraphs::builder().build().start().await;
    let router = TestRouter::builder()
        .with_subgraphs(&subgraphs)
        .inline_config(format!(
            r#"
                supergraph:
                  source: file
                  path: supergraph.graphql
                http:
                  graphql_endpoint: /custom
                persisted_documents:
                  enabled: true
                  require_id: true
                  storage:
                    type: file
                    path: "{}"
                  extractors:
                    - type: url_path_param
                      template: /docs/:id
                "#,
            manifest.path().display(),
        ))
        .build()
        .start()
        .await;

    let response = router
        .send_post_request(&format!("/custom/docs/{PATH_DOC_ID}"), json!({}), None)
        .await;

    assert_resolves_successfully(response).await;
}

#[ntex::test]
async fn uses_first_match_with_other_extractors() {
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
                    - type: url_path_param
                      template: /docs/:id
                    - type: url_query_param
                      name: documentId
                "#,
            manifest.path().display(),
        ))
        .build()
        .start()
        .await;

    let response = router
        .send_post_request(
            &format!("/graphql/docs/{PATH_DOC_ID}?documentId=sha256%3Anotfound"),
            json!({}),
            None,
        )
        .await;

    assert_resolves_successfully(response).await;
}

#[ntex::test]
// Verifies queryless GET path requests can resolve persisted document id from url_path_param extractor.
async fn resolves_id_from_queryless_get_path() {
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
                    - type: url_path_param
                      template: /docs/:id
                "#,
            manifest.path().display(),
        ))
        .build()
        .start()
        .await;

    let response = router
        .serv()
        .get(&format!("/graphql/docs/{PATH_DOC_ID}"))
        .send()
        .await
        .expect("failed to send graphql request");

    assert_resolves_successfully(response).await;
}
