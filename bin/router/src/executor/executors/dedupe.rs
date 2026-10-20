use ahash::RandomState;
use http::HeaderMap;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use xxhash_rust::xxh3::{xxh3_64, xxh3_64_with_seed, Xxh3};

use crate::executor::executors::http::SendRequestOpts;

impl Hash for SendRequestOpts<'_> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.method.hash(state);
        self.endpoint.hash(state);
        hash_headers(&self.headers).hash(state);
        self.body.hash(state);
    }
}

/// Hashes the headers the same way in any order, without sorting them into a map first.
/// Header names are lowercase already, so each name + value pair gets its own hash
/// and the hashes are summed. Values that aren't visible ASCII are skipped.
fn hash_headers(headers: &HeaderMap) -> (u64, usize) {
    let mut sum: u64 = 0;
    let mut count = 0;
    for (name, value) in headers.iter() {
        if let Ok(value) = value.to_str() {
            let pair = xxh3_64_with_seed(value.as_bytes(), xxh3_64(name.as_str().as_bytes()));
            sum = sum.wrapping_add(pair);
            count += 1;
        }
    }
    (sum, count)
}

impl SendRequestOpts<'_> {
    /// Generate a hash for the request options, used for deduplication.
    pub fn fingerprint(&self) -> u64 {
        let mut hasher = Xxh3::new();
        self.hash(&mut hasher);
        hasher.finish()
    }
}

static LEADER_COUNTER: AtomicU64 = AtomicU64::new(1);
static LEADER_SALT: OnceLock<u64> = OnceLock::new();

/// Generate a unique fingerprint for the current leader.
/// This is used to identify the leader in a distributed system.
pub fn unique_leader_fingerprint() -> u64 {
    let idx = LEADER_COUNTER.fetch_add(1, Ordering::Relaxed);
    let salt =
        LEADER_SALT.get_or_init(|| RandomState::new().hash_one(b"unique-leader-fingerprint"));
    idx ^ salt
}

#[cfg(test)]
mod tests {
    use super::hash_headers;
    use http::{HeaderMap, HeaderName, HeaderValue};

    fn headers(pairs: &[(&'static str, &'static str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.append(
                HeaderName::from_static(name),
                HeaderValue::from_static(value),
            );
        }
        map
    }

    #[test]
    fn header_order_does_not_change_the_hash() {
        assert_eq!(
            hash_headers(&headers(&[("a", "1"), ("b", "2")])),
            hash_headers(&headers(&[("b", "2"), ("a", "1")]))
        );
    }

    #[test]
    fn every_header_value_counts() {
        let base = hash_headers(&headers(&[("a", "1"), ("b", "2")]));
        assert_ne!(base, hash_headers(&headers(&[("a", "1"), ("b", "3")])));
        assert_ne!(base, hash_headers(&headers(&[("a", "2"), ("b", "1")])));
        // repeated headers: all values count, not just the last one
        assert_ne!(
            hash_headers(&headers(&[("a", "1"), ("a", "2")])),
            hash_headers(&headers(&[("a", "3"), ("a", "2")]))
        );
    }
}
