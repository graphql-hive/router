use sonic_rs::{json, JsonValueTrait};
use tempfile::NamedTempFile;

use crate::testkit::ClientResponseExt;

pub(super) const DOC_ID: &str = "sha256:abc123";
pub(super) const PATH_DOC_ID: &str = "abc-123";
pub(super) const DOC_QUERY: &str = "{ topProducts { name } }";

pub(super) fn write_manifest() -> NamedTempFile {
    let file = NamedTempFile::new().expect("failed to create temp persisted document manifest");
    std::fs::write(
        file.path(),
        sonic_rs::to_string(&json!({
            DOC_ID: DOC_QUERY,
            PATH_DOC_ID: DOC_QUERY,
        }))
        .expect("failed to serialize persisted document manifest"),
    )
    .expect("failed to write persisted document manifest");
    file
}

pub(super) async fn assert_resolves_successfully(response: ntex::client::ClientResponse) {
    assert!(response.status().is_success(), "expected 2xx response");
    let body = response.json_body().await;
    assert!(body["errors"].is_null(), "unexpected graphql errors: {body}");
    assert!(
        body["data"]["topProducts"].is_array(),
        "expected resolved persisted query data: {body}"
    );
}

pub(super) async fn assert_error_code(response: ntex::client::ClientResponse, code: &str) {
    let body = response.json_body().await;
    let got = body["errors"][0]["extensions"]["code"]
        .as_str()
        .expect("expected graphql error code string");
    assert_eq!(got, code, "unexpected response body: {body}");
}
