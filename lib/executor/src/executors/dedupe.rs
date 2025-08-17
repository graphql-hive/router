use ahash::AHasher;
use bytes::Bytes;
use http::{HeaderMap, Method, StatusCode, Uri};
use std::collections::BTreeMap;
use std::hash::{BuildHasherDefault, Hash, Hasher};

#[derive(Debug, Clone)]
pub struct SharedResponse {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Bytes,
}

#[derive(Debug, Clone, Eq)]
pub struct RequestFingerprint {
    method: Method,
    url: Uri,
    /// BTreeMap to ensure case-insensitivity and consistent order for hashing
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

impl RequestFingerprint {
    pub fn new(
        method: &Method,
        url: &Uri,
        req_headers: &HeaderMap,
        body_bytes: &[u8],
        fingerprint_headers: &[String],
    ) -> Self {
        let mut headers = BTreeMap::new();
        if fingerprint_headers.is_empty() {
            // fingerprint all headers
            for (key, value) in req_headers.iter() {
                if let Ok(value_str) = value.to_str() {
                    headers.insert(key.as_str().to_lowercase(), value_str.to_string());
                }
            }
        } else {
            for header_name in fingerprint_headers.iter() {
                if let Some(value) = req_headers.get(header_name) {
                    if let Ok(value_str) = value.to_str() {
                        headers.insert(header_name.to_lowercase(), value_str.to_string());
                    }
                }
            }
        }

        Self {
            method: method.clone(),
            url: url.clone(),
            headers,
            body: body_bytes.to_vec(),
        }
    }
}

impl Hash for RequestFingerprint {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.method.hash(state);
        self.url.hash(state);
        self.headers.hash(state);
        self.body.hash(state);
    }
}

impl PartialEq for RequestFingerprint {
    fn eq(&self, other: &Self) -> bool {
        self.method == other.method
            && self.url == other.url
            && self.headers == other.headers
            && self.body == other.body
    }
}

pub type ABuildHasher = BuildHasherDefault<AHasher>;
