use ahash::AHasher;
use bytes::Bytes;
use http::{HeaderMap, Method, StatusCode, Uri};
use std::collections::BTreeMap;
use std::hash::{BuildHasher, BuildHasherDefault, Hash, Hasher};

#[derive(Debug, Clone)]
pub struct SharedResponse {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Bytes,
}

pub fn request_fingerprint(
    method: &Method,
    url: &Uri,
    req_headers: &HeaderMap,
    body_bytes: &[u8],
    fingerprint_headers: &[String],
) -> u64 {
    let build_hasher = ABuildHasher::default();
    let mut hasher = build_hasher.build_hasher();

    // BTreeMap to ensure case-insensitivity and consistent order for hashing
    let mut headers = BTreeMap::new();
    if fingerprint_headers.is_empty() {
        // fingerprint all headers
        for (key, value) in req_headers.iter() {
            if let Ok(value_str) = value.to_str() {
                headers.insert(key.as_str(), value_str);
            }
        }
    } else {
        for header_name in fingerprint_headers.iter() {
            if let Some(value) = req_headers.get(header_name) {
                if let Ok(value_str) = value.to_str() {
                    headers.insert(header_name, value_str);
                }
            }
        }
    }

    method.hash(&mut hasher);
    url.hash(&mut hasher);
    headers.hash(&mut hasher);
    body_bytes.hash(&mut hasher);

    hasher.finish()
}

pub type ABuildHasher = BuildHasherDefault<AHasher>;
