use std::time::Duration;

use crate::{
    persisted_documents::shared::{assert_resolves_successfully, create_manifest, DOC_ID},
    testkit::{s3_mock::S3Mock, ClientResponseExt, TestRouter, TestSubgraphs},
};
use sonic_rs::{json, JsonValueTrait};

#[ntex::test]
async fn should_load_persisted_documents_from_s3() {
    let subgraphs = TestSubgraphs::builder().build().start().await;
    let storage = S3Mock::start("test-bucket").await;
    let location = "persisted/manifest.json";
    storage.set(location, create_manifest().as_bytes()).await;

    let router = TestRouter::builder()
        .with_subgraphs(&subgraphs)
        .inline_config(format!(
            r#"
                supergraph:
                  source: file
                  path: supergraph.graphql
                storages:
                  test-s3:
                    type: s3
                    bucket: {}
                    endpoint: {}
                    allow_http: true
                    credentials:
                      type: static
                      access_key_id: {}
                      secret_access_key: {}
                persisted_documents:
                  enabled: true
                  require_id: true
                  storage:
                    type: storage
                    storage_id: test-s3
                    location: {}
                "#,
            storage.bucket(),
            storage.url(),
            storage.access_key(),
            storage.secret_key(),
            location
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
}

// When persisted docs store is not available at all, service startup should fail
#[ntex::test]
#[should_panic]
async fn should_fail_when_store_not_available() {
    TestRouter::builder()
        .inline_config(format!(
            r#"
                supergraph:
                  source: file
                  path: supergraph.graphql
                storages:
                  test-s3:
                    type: s3
                    bucket: test
                    endpoint: http://localhost:1111
                    allow_http: true
                    credentials:
                      type: static
                      access_key_id: test
                      secret_access_key: dummy
                persisted_documents:
                  enabled: true
                  require_id: true
                  storage:
                    type: storage
                    storage_id: test-s3
                    location: file.json
                "#,
        ))
        .build()
        .start()
        .await;
}

#[ntex::test]
async fn should_reload_persisted_documents_from_s3() {
    let subgraphs = TestSubgraphs::builder().build().start().await;
    let storage = S3Mock::start("test-bucket").await;
    let location = "persisted/manifest.json";
    storage.set(location, create_manifest().as_bytes()).await;

    let router = TestRouter::builder()
        .with_subgraphs(&subgraphs)
        .inline_config(format!(
            r#"
                supergraph:
                  source: file
                  path: supergraph.graphql
                storages:
                  test-s3:
                    type: s3
                    bucket: {}
                    endpoint: {}
                    allow_http: true
                    credentials:
                      type: static
                      access_key_id: {}
                      secret_access_key: {}
                persisted_documents:
                  enabled: true
                  require_id: true
                  storage:
                    type: storage
                    storage_id: test-s3
                    location: {}
                    poll_interval: 100ms
                "#,
            storage.bucket(),
            storage.url(),
            storage.access_key(),
            storage.secret_key(),
            location
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

    let new_doc_id = "xyz-100";
    storage
        .set(
            location,
            sonic_rs::to_string(&json!({
                new_doc_id: "{ me { id } }",
            }))
            .expect("fail to serialize string")
            .as_bytes(),
        )
        .await;

    tokio::time::sleep(Duration::from_millis(150)).await;

    let response = router
        .send_post_request("/graphql", json!({ "documentId": new_doc_id }), None)
        .await;
    assert!(response.status().is_success(), "expected 2xx response");
    let body = response.json_body().await;
    assert!(
        body["errors"].is_null(),
        "unexpected graphql errors: {body}"
    );
    assert!(
        body["data"]["me"].is_object(),
        "expected resolved persisted query data: {body}"
    );

    // The old ID should not work any longer because store has changed
    let response = router
        .send_post_request("/graphql", json!({ "documentId": DOC_ID }), None)
        .await;

    assert!(response.status().is_client_error(), "expected 4xx response");
}

#[ntex::test]
async fn should_still_work_if_storage_failed_after() {
    let subgraphs = TestSubgraphs::builder().build().start().await;
    let storage = S3Mock::start("test-bucket").await;
    let location = "persisted/manifest.json";
    storage.set(location, create_manifest().as_bytes()).await;

    let router = TestRouter::builder()
        .with_subgraphs(&subgraphs)
        .inline_config(format!(
            r#"
                supergraph:
                  source: file
                  path: supergraph.graphql
                storages:
                  test-s3:
                    type: s3
                    bucket: {}
                    endpoint: {}
                    allow_http: true
                    credentials:
                      type: static
                      access_key_id: {}
                      secret_access_key: {}
                persisted_documents:
                  enabled: true
                  require_id: true
                  storage:
                    type: storage
                    storage_id: test-s3
                    location: {}
                    poll_interval: 100ms
                "#,
            storage.bucket(),
            storage.url(),
            storage.access_key(),
            storage.secret_key(),
            location
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

    // Stop the s3 service
    drop(storage);

    // Wait a bit for the poller to detect the store is unavailable
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Runnign a query should still work
    let response = router
        .send_post_request("/graphql", json!({ "documentId": DOC_ID }), None)
        .await;

    assert_resolves_successfully(response).await;
}
