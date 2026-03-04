use ahash::AHasher;
use ahash::RandomState;
use std::collections::BTreeMap;
use std::hash::{BuildHasherDefault, Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use xxhash_rust::xxh3::Xxh3;

use crate::executors::http::SendRequestOpts;

impl Hash for SendRequestOpts<'_> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // BTreeMap to ensure case-insensitivity and consistent order for hashing
        let mut headers = BTreeMap::new();
        for (header_name, header_value) in self.headers.iter() {
            if let Ok(value_str) = header_value.to_str() {
                headers.insert(header_name.as_str(), value_str);
            }
        }

        self.method.hash(state);
        self.endpoint.hash(state);
        headers.hash(state);
        self.body.hash(state);
    }
}

impl SendRequestOpts<'_> {
    /// Generate a hash for the request options, used for deduplication.
    pub fn fingerprint(&self) -> u64 {
        let mut hasher = Xxh3::new();
        self.hash(&mut hasher);
        hasher.finish()
    }
}

pub type ABuildHasher = BuildHasherDefault<AHasher>;

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
