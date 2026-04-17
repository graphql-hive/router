use std::time::Duration;

use sonic_rs::json;

use super::shared::{assert_error_code, assert_resolves_successfully};
use crate::testkit::{some_header_map, TestRouter, TestSubgraphs};

mod negative_cache {
    use super::*;

    const MISSING_DOC_ID: &str = "app~1.0.0~missing-doc";
    const MISSING_DOC_CDN_PATH: &str = "/apps/app/1.0.0/missing-doc";

    #[ntex::test]
    // Verifies that negative cache is enabled by default for Hive storage.
    // Expects that the second request for same missing id within default TTL avoids a second CDN fetch.
    async fn default_skips_second_miss_within_ttl() {
        let mut server = mockito::Server::new_async().await;
        let host = server.host_with_port();
        let miss = server
            .mock("GET", MISSING_DOC_CDN_PATH)
            .expect(1)
            .match_header("x-hive-cdn-key", "dummy_key")
            .with_status(404)
            .create();

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
                    type: hive
                    endpoint: http://{host}
                    key: dummy_key
                    retry_policy:
                      max_retries: 0
            "#
            ))
            .build()
            .start()
            .await;

        let response = router
            .send_post_request("/graphql", json!({ "documentId": MISSING_DOC_ID }), None)
            .await;
        assert_error_code(response, "PERSISTED_DOCUMENT_NOT_FOUND").await;

        let response = router
            .send_post_request("/graphql", json!({ "documentId": MISSING_DOC_ID }), None)
            .await;
        assert_error_code(response, "PERSISTED_DOCUMENT_NOT_FOUND").await;

        miss.assert();
    }

    #[ntex::test]
    // Verifies that explicitly disabling negative cache forces misses to hit CDN each time.
    // Expects repeated requests for same missing id trigger a CDN fetch each time.
    async fn disabled_retries_each_miss() {
        let mut server = mockito::Server::new_async().await;
        let host = server.host_with_port();
        let miss = server
            .mock("GET", MISSING_DOC_CDN_PATH)
            .expect(2)
            .match_header("x-hive-cdn-key", "dummy_key")
            .with_status(404)
            .create();

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
                    type: hive
                    endpoint: http://{host}
                    key: dummy_key
                    retry_policy:
                      max_retries: 0
                    negative_cache: false
            "#
            ))
            .build()
            .start()
            .await;

        let response = router
            .send_post_request("/graphql", json!({ "documentId": MISSING_DOC_ID }), None)
            .await;
        assert_error_code(response, "PERSISTED_DOCUMENT_NOT_FOUND").await;

        let response = router
            .send_post_request("/graphql", json!({ "documentId": MISSING_DOC_ID }), None)
            .await;
        assert_error_code(response, "PERSISTED_DOCUMENT_NOT_FOUND").await;

        miss.assert();
    }

    #[ntex::test]
    // Verifies that negative cache entries expire after configured TTL.
    // Expects that the same missing id triggers CDN fetch again after TTL has elapsed.
    async fn expires_and_refetches_after_ttl() {
        let mut server = mockito::Server::new_async().await;
        let host = server.host_with_port();
        let miss = server
            .mock("GET", MISSING_DOC_CDN_PATH)
            .expect(2)
            .match_header("x-hive-cdn-key", "dummy_key")
            .with_status(404)
            .create();

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
                    type: hive
                    endpoint: http://{host}
                    key: dummy_key
                    retry_policy:
                      max_retries: 0
                    negative_cache:
                      ttl: 100ms
            "#
            ))
            .build()
            .start()
            .await;

        let response = router
            .send_post_request("/graphql", json!({ "documentId": MISSING_DOC_ID }), None)
            .await;
        assert_error_code(response, "PERSISTED_DOCUMENT_NOT_FOUND").await;

        tokio::time::sleep(Duration::from_millis(250)).await;

        let response = router
            .send_post_request("/graphql", json!({ "documentId": MISSING_DOC_ID }), None)
            .await;
        assert_error_code(response, "PERSISTED_DOCUMENT_NOT_FOUND").await;

        miss.assert();
    }

    #[ntex::test]
    // Verifies that enabling negative cache with boolean true uses default cache configuration.
    // Expects the second request for same missing id within default TTL to avoid a second CDN fetch.
    async fn enabled_uses_default_ttl() {
        let mut server = mockito::Server::new_async().await;
        let host = server.host_with_port();
        let miss = server
            .mock("GET", MISSING_DOC_CDN_PATH)
            .expect(1)
            .match_header("x-hive-cdn-key", "dummy_key")
            .with_status(404)
            .create();

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
                    type: hive
                    endpoint: http://{host}
                    key: dummy_key
                    retry_policy:
                      max_retries: 0
                    negative_cache: true
            "#
            ))
            .build()
            .start()
            .await;

        let response = router
            .send_post_request("/graphql", json!({ "documentId": MISSING_DOC_ID }), None)
            .await;
        assert_error_code(response, "PERSISTED_DOCUMENT_NOT_FOUND").await;

        let response = router
            .send_post_request("/graphql", json!({ "documentId": MISSING_DOC_ID }), None)
            .await;
        assert_error_code(response, "PERSISTED_DOCUMENT_NOT_FOUND").await;

        miss.assert();
    }
}

#[ntex::test]
// Verifies successful Hive lookups are cached in memory by default.
// Expects that two router requests for the same document id produce only one CDN fetch.
async fn reuses_cached_document_on_second_request() {
    let doc_id: &str = "app~1.0.0~found-doc";
    let cdn_path: &str = "/apps/app/1.0.0/found-doc";

    let mut server = mockito::Server::new_async().await;
    let host = server.host_with_port();
    let hit = server
        .mock("GET", cdn_path)
        .expect(1)
        .match_header("x-hive-cdn-key", "dummy_key")
        .with_status(200)
        .with_header("content-type", "text/plain")
        .with_body("{ topProducts { name } }")
        .create();

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
                    type: hive
                    endpoint: http://{host}
                    key: dummy_key
                    retry_policy:
                      max_retries: 0
            "#
        ))
        .build()
        .start()
        .await;

    let response = router
        .send_post_request("/graphql", json!({ "documentId": doc_id }), None)
        .await;
    assert_resolves_successfully(response).await;

    let response = router
        .send_post_request("/graphql", json!({ "documentId": doc_id }), None)
        .await;
    assert_resolves_successfully(response).await;

    hit.assert();
}

#[ntex::test]
// Verifies app-qualified and header-qualified forms of the same document id share one cache key.
// Expects the second router request to reuse the first fetch and avoid a second CDN hit.
async fn caches_documents_by_id() {
    let app_doc_id: &str = "app~1.0.0~found-doc";
    let plain_doc_id: &str = "found-doc";
    let cdn_path: &str = "/apps/app/1.0.0/found-doc";

    let mut server = mockito::Server::new_async().await;
    let host = server.host_with_port();
    let hit = server
        .mock("GET", cdn_path)
        .expect(1)
        .match_header("x-hive-cdn-key", "dummy_key")
        .with_status(200)
        .with_header("content-type", "text/plain")
        .with_body("{ topProducts { name } }")
        .create();

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
                    type: hive
                    endpoint: http://{host}
                    key: dummy_key
                    retry_policy:
                      max_retries: 0
            "#
        ))
        .build()
        .start()
        .await;

    let response = router
        .send_post_request("/graphql", json!({ "documentId": app_doc_id }), None)
        .await;
    assert_resolves_successfully(response).await;

    let response = router
        .send_post_request(
            "/graphql",
            json!({ "documentId": plain_doc_id }),
            some_header_map!(
                ::http::header::HeaderName::from_static("graphql-client-name") => "app",
                ::http::header::HeaderName::from_static("graphql-client-version") => "1.0.0",
            ),
        )
        .await;
    assert_resolves_successfully(response).await;

    hit.assert();
}

#[ntex::test]
// Verifies concurrent requests for the same logical document id reuse a single CDN fetch.
// Expects one request to Hive CDN when one app-qualified and one header-qualified request hit router concurrently.
async fn concurrent_requests_share_one_cdn_fetch() {
    let app_doc_id: &str = "app~1.0.0~found-doc";
    let plain_doc_id: &str = "found-doc";
    let cdn_path: &str = "/apps/app/1.0.0/found-doc";

    let mut server = mockito::Server::new_async().await;
    let host = server.host_with_port();
    let hit = server
        .mock("GET", cdn_path)
        .expect(1)
        .match_header("x-hive-cdn-key", "dummy_key")
        .with_status(200)
        .with_header("content-type", "text/plain")
        .with_chunked_body(|writer| {
            std::thread::sleep(Duration::from_millis(150));
            writer.write_all(b"{ topProducts { name } }")
        })
        .create();

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
                    type: hive
                    endpoint: http://{host}
                    key: dummy_key
                    retry_policy:
                      max_retries: 0
            "#
        ))
        .build()
        .start()
        .await;

    let app_id_request =
        router.send_post_request("/graphql", json!({ "documentId": app_doc_id }), None);
    let plain_id_request = router.send_post_request(
        "/graphql",
        json!({ "documentId": plain_doc_id }),
        some_header_map!(
            ::http::header::HeaderName::from_static("graphql-client-name") => "app",
            ::http::header::HeaderName::from_static("graphql-client-version") => "1.0.0",
        ),
    );

    let (app_id_response, plain_id_response) = tokio::join!(app_id_request, plain_id_request);

    assert_resolves_successfully(app_id_response).await;
    assert_resolves_successfully(plain_id_response).await;

    hit.assert();
}
