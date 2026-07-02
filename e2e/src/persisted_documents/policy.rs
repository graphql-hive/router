use sonic_rs::json;

use super::shared::{assert_error_code, assert_resolves_successfully, write_manifest};
use crate::testkit::{some_header_map, EnvVarsGuard, TestRouter, TestSubgraphs};

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

#[ntex::test]
async fn require_id_expression_basic() {
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
                  require_id:
                    expression: .request.headers."x-require-id" == "true"
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
                "query": "{ topProducts { name } }"
            }),
            some_header_map! {
                http::header::HeaderName::from_static("x-require-id") => "true"
            },
        )
        .await;

    assert_error_code(response, "PERSISTED_DOCUMENT_ID_REQUIRED").await;

    let response = router
        .send_post_request(
            "/graphql",
            json!({
                "query": "{ topProducts { name } }"
            }),
            some_header_map! {
                http::header::HeaderName::from_static("x-require-id") => "false"
            },
        )
        .await;

    assert_resolves_successfully(response).await;
}

#[ntex::test]
async fn require_id_expression_with_env_secret() {
    let _env_guard = EnvVarsGuard::new()
        .set("BYPASS_SECRET", "bypass123")
        .apply()
        .await;

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
                  require_id:
                    expression: is_null(env("BYPASS_SECRET")) || .request.headers."x-bypass-require-id" != env("BYPASS_SECRET")
                  storage:
                    type: file
                    path: "{}"
                "#,
            manifest.path().display(),
        ))
        .build()
        .start()
        .await;

    // Header does not match the env secret.
    // require_id is true
    let response = router
        .send_post_request(
            "/graphql",
            json!({
                "query": "{ topProducts { name } }"
            }),
            some_header_map! {
                http::header::HeaderName::from_static("x-bypass-require-id") => "wrong-secret"
            },
        )
        .await;
    assert_error_code(response, "PERSISTED_DOCUMENT_ID_REQUIRED").await;

    // Header matches the env secret.
    // require_id is false
    let response = router
        .send_post_request(
            "/graphql",
            json!({
                "query": "{ topProducts { name } }"
            }),
            some_header_map! {
                http::header::HeaderName::from_static("x-bypass-require-id") => "bypass123"
            },
        )
        .await;

    assert_resolves_successfully(response).await;

    // Env var not set.
    // is_null(env(...)) is true, so require_id is true
    let response = router
        .send_post_request(
            "/graphql",
            json!({
                "query": "{ topProducts { name } }"
            }),
            None,
        )
        .await;

    assert_error_code(response, "PERSISTED_DOCUMENT_ID_REQUIRED").await;
}
