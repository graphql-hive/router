use ahash::RandomState;
use http::{HeaderMap, HeaderName};
use smallvec::SmallVec;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use xxhash_rust::xxh3::Xxh3;

use crate::executor::executors::http::SendRequestOpts;

impl Hash for SendRequestOpts<'_> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.method.hash(state);
        self.endpoint.hash(state);
        hash_headers(&self.headers, state);
        self.body.hash(state);
    }
}

fn hash_headers<H: Hasher>(headers: &HeaderMap, state: &mut H) {
    let unique_names = headers.len() == headers.keys_len();
    unique_names.hash(state);
    headers.keys_len().hash(state);

    let mut names: SmallVec<[&HeaderName; 16]> = headers.keys().collect();
    names.sort_unstable_by(|left, right| left.as_str().cmp(right.as_str()));

    for name in names {
        name.hash(state);
        if unique_names {
            1_usize.hash(state);
            headers
                .get(name)
                .expect("a header key always has a value")
                .hash(state);
        } else {
            let values = headers.get_all(name);
            values.iter().count().hash(state);
            for value in values.iter() {
                value.hash(state);
            }
        }
    }
}

impl SendRequestOpts<'_> {
    /// Generate a hash for the request options, used for deduplication.
    pub fn fingerprint(&self) -> u128 {
        let mut hasher = Xxh3::new();
        self.hash(&mut hasher);
        hasher.digest128()
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
    use xxhash_rust::xxh3::Xxh3;

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
            header_hash(&headers(&[("a", "1"), ("b", "2")])),
            header_hash(&headers(&[("b", "2"), ("a", "1")]))
        );
    }

    #[test]
    fn changing_other_name_order_keeps_repeated_value_order() {
        assert_eq!(
            header_hash(&headers(&[("a", "first"), ("b", "x"), ("a", "second")])),
            header_hash(&headers(&[("b", "x"), ("a", "first"), ("a", "second")]))
        );
    }

    fn header_hash(headers: &HeaderMap) -> u128 {
        let mut hasher = Xxh3::new();
        hash_headers(headers, &mut hasher);
        hasher.digest128()
    }
}
