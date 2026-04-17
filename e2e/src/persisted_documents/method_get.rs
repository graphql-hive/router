use super::shared::{assert_error_code, assert_resolves_successfully, write_manifest};
use crate::testkit::{TestRouter, TestSubgraphs};

#[ntex::test]
// Verifies GET requests can resolve persisted documents via the default documentId query parameter.
async fn resolves_from_document_id_query_param() {
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
        .get("/graphql?documentId=sha256%3Aabc123")
        .send()
        .await
        .expect("failed to send graphql request");

    assert_resolves_successfully(response).await;
}

#[ntex::test]
// Verifies GET requests can resolve persisted documents through a configured custom query parameter.
async fn resolves_from_custom_query_param_extractor() {
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
                    - type: url_query_param
                      name: pid
                "#,
            manifest.path().display(),
        ))
        .build()
        .start()
        .await;

    let response = router
        .serv()
        .get("/graphql?pid=sha256:abc123")
        .send()
        .await
        .expect("failed to send graphql request");

    assert_resolves_successfully(response).await;
}

#[ntex::test]
// Verifies GET requests with an empty query and no persisted document id do not resolve a document
async fn requires_id_when_query_is_empty() {
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
        .get("/graphql?query=")
        .send()
        .await
        .expect("failed to send graphql request");

    assert_error_code(response, "PERSISTED_DOCUMENT_ID_REQUIRED").await;
}

#[ntex::test]
// Verifies queryless GET request is possible
async fn requires_id_when_queryless_get_has_no_id() {
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
        .get("/graphql")
        .send()
        .await
        .expect("failed to send graphql request");

    assert_error_code(response, "PERSISTED_DOCUMENT_ID_REQUIRED").await;
}

#[ntex::test]
// Verifies percent-encoded values in the configured custom query parameter are decoded before lookup
async fn decodes_percent_encoded_custom_param() {
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
                    - type: url_query_param
                      name: pid
                "#,
            manifest.path().display(),
        ))
        .build()
        .start()
        .await;

    let response = router
        .serv()
        .get("/graphql?pid=sha256%3Aabc123")
        .send()
        .await
        .expect("failed to send graphql request");

    assert_resolves_successfully(response).await;
}

#[ntex::test]
// Verifies a configured custom query parameter extractor does not fall back to default `documentId`
async fn requires_id_when_custom_param_is_missing() {
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
                    - type: url_query_param
                      name: pid
                "#,
            manifest.path().display(),
        ))
        .build()
        .start()
        .await;

    let response = router
        .serv()
        .get("/graphql?documentId=sha256%3Aabc123")
        .send()
        .await
        .expect("failed to send graphql request");

    assert_error_code(response, "PERSISTED_DOCUMENT_ID_REQUIRED").await;
}
