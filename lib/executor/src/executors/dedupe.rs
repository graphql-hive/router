use ahash::AHasher;
use http::{HeaderMap, Method, Uri};
use std::collections::BTreeMap;
use std::hash::{BuildHasher, BuildHasherDefault, Hash, Hasher};

pub fn request_fingerprint(
    method: &Method,
    url: &Uri,
    req_headers: &HeaderMap,
    body_bytes: &[u8],
) -> u64 {
    let build_hasher = ABuildHasher::default();
    let mut hasher = build_hasher.build_hasher();

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
