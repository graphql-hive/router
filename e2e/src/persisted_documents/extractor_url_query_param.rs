use sonic_rs::json;

use super::shared::{assert_error_code, assert_resolves_successfully, write_manifest};
use crate::testkit::{TestRouter, TestSubgraphs};

#[ntex::test]
// Make sure url_query_param extractor does not error on missing query string,
// but returns none instead
async fn missing_query_string_returns_none() {
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
        .post("/graphql")
        .send_json(&json!({}))
        .await
        .expect("failed to send graphql request");

    assert_error_code(response, "PERSISTED_DOCUMENT_ID_REQUIRED").await;
}

#[ntex::test]
// Make sure url_query_param extractor decodes percent-encoded values correctly
async fn decodes_percent_encoded_value() {
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
                    - type: url_query_param
                      name: documentId
                "#,
            manifest.path().display(),
        ))
        .build()
        .start()
        .await;

    let response = router
        .send_post_request("/graphql?documentId=sha256%3Aabc123", json!({}), None)
        .await;

    assert_resolves_successfully(response).await;
}

#[ntex::test]
// url_query_param extractor uses first match
async fn uses_first_value_for_duplicate_keys() {
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
            // first: correct, second: incorrect
            "/graphql?documentId=sha256%3Aabc123&documentId=sha256%3Aother",
            json!({}),
            None,
        )
        .await;

    assert_resolves_successfully(response).await;
}

#[ntex::test]
// url_query_param extractor matches first param, even if it's empty
async fn first_empty_match() {
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
            // documentId=&
            "/graphql?documentId=&documentId=sha256%3Aabc123",
            json!({}),
            None,
        )
        .await;

    assert_error_code(response, "PERSISTED_DOCUMENT_ID_REQUIRED").await;

    let response = router
        .send_post_request(
            // documentId&
            "/graphql?documentId&documentId=sha256%3Aabc123",
            json!({}),
            None,
        )
        .await;

    assert_error_code(response, "PERSISTED_DOCUMENT_ID_REQUIRED").await;
}

#[ntex::test]
// url_query_param matches exactly the param name, not a prefix/suffix
async fn ignores_prefix_matches_and_continues() {
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
                    - type: url_query_param
                      name: key
                "#,
            manifest.path().display(),
        ))
        .build()
        .start()
        .await;

    let response = router
        .send_post_request("/graphql?keys=1&key=sha256%3Aabc123", json!({}), None)
        .await;

    assert_resolves_successfully(response).await;

    let response = router
        .send_post_request("/graphql?skey=1&key=sha256%3Aabc123", json!({}), None)
        .await;

    assert_resolves_successfully(response).await;
}
