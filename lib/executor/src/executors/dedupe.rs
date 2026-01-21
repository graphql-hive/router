use ahash::AHasher;
use bytes::Bytes;
use hive_router_internal::telemetry::otel::opentelemetry::trace::SpanContext;
use http::{HeaderMap, Method, StatusCode, Uri};
use std::collections::BTreeMap;
use std::hash::{BuildHasherDefault, Hash, Hasher};
use xxhash_rust::xxh3::Xxh3;

#[derive(Debug, Clone)]
pub struct SharedResponse {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Bytes,
    pub span_context: SpanContext,
}

pub fn request_fingerprint(
    method: &Method,
    url: &Uri,
    req_headers: &HeaderMap,
    body_bytes: &[u8],
) -> u64 {
    // In e2e tests, we want to be able to compare request fingerprints,
    // and that requires this function to produce the same hash for the same input,
    // between test runs (between processes).
    // We used to have ahash::AHasher, but it was using random seeds every time it was created.
    // It prevented consistent hashing between test runs.
    let mut hasher = Xxh3::new();

    // BTreeMap to ensure case-insensitivity and consistent order for hashing
    let mut headers = BTreeMap::new();
    for (header_name, header_value) in req_headers.iter() {
        if let Ok(value_str) = header_value.to_str() {
            headers.insert(header_name.as_str(), value_str);
        }
    }

    method.hash(&mut hasher);
    url.hash(&mut hasher);
    headers.hash(&mut hasher);
    body_bytes.hash(&mut hasher);

    hasher.finish()
}

pub type ABuildHasher = BuildHasherDefault<AHasher>;
