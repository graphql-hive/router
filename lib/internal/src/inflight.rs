use std::{
    future::Future,
    hash::{BuildHasher, BuildHasherDefault, Hash},
    sync::Arc,
};

use ahash::AHasher;
use dashmap::{mapref::entry::Entry, DashMap};
use tokio::sync::OnceCell;

pub type ABuildHasher = BuildHasherDefault<AHasher>;
type InFlightValue<V> = Arc<V>;
type InFlightCell<V> = Arc<OnceCell<InFlightValue<V>>>;
type InFlightInnerMap<K, V, S> = DashMap<K, InFlightCell<V>, S>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InFlightRole {
    Leader,
    Joiner,
}

pub struct InFlightMap<K, V, S = ABuildHasher> {
    inner: Arc<InFlightInnerMap<K, V, S>>,
}

impl<K, V, S> Clone for InFlightMap<K, V, S> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<K, V> Default for InFlightMap<K, V, ABuildHasher>
where
    K: Eq + Hash,
{
    fn default() -> Self {
        Self::with_hasher(ABuildHasher::default())
    }
}

impl<K, V, S> InFlightMap<K, V, S>
where
    K: Eq + Hash,
    S: BuildHasher + Clone,
{
    #[inline]
    pub fn with_hasher(hasher: S) -> Self {
        Self {
            inner: Arc::new(DashMap::with_hasher(hasher)),
        }
    }

    #[inline]
    pub fn claim(&self, key: K) -> InFlightClaim<K, V, S>
    where
        K: Clone,
    {
        match self.inner.entry(key.clone()) {
            Entry::Occupied(entry) => InFlightClaim {
                key,
                cell: entry.get().clone(),
                map: self.clone(),
            },
            Entry::Vacant(entry) => {
                let cell = Arc::new(OnceCell::new());
                entry.insert(cell.clone());
                InFlightClaim {
                    key,
                    cell,
                    map: self.clone(),
                }
            }
        }
    }

    #[inline]
    pub fn remove(&self, key: &K) {
        self.inner.remove(key);
    }
}

pub struct InFlightClaim<K, V, S = ABuildHasher> {
    key: K,
    cell: InFlightCell<V>,
    map: InFlightMap<K, V, S>,
}

impl<K, V, S> InFlightClaim<K, V, S>
where
    K: Eq + Hash + Clone,
    S: BuildHasher + Clone,
{
    /// Initialises the cell if empty (leader) or waits for the existing value (joiner).
    ///
    /// The leader's `init` closure receives an `InFlightCleanupGuard`. Dropping the guard removes
    /// the entry from the map. For short-lived work (queries) drop it immediately. For long-lived
    /// work (subscriptions) move it into the task that owns the upstream so the entry stays
    /// visible to joiners for the full lifetime of the stream.
    ///
    /// On init failure the entry is cleaned up automatically regardless of what the caller does
    /// with the guard, so no entry is left dangling.
    ///
    /// Joiners do not invoke `init` - they share the already-initialised value and have no cleanup
    /// responsibility.
    #[inline]
    pub async fn get_or_try_init<E, F, Fut>(self, init: F) -> Result<(Arc<V>, InFlightRole), E>
    where
        F: FnOnce(InFlightCleanupGuard<K, V, S>) -> Fut,
        Fut: Future<Output = Result<V, E>>,
    {
        let mut did_initialize = false;
        let key = self.key.clone();
        let map = self.map.clone();

        let value = self
            .cell
            .get_or_try_init(|| {
                did_initialize = true;
                let guard = InFlightCleanupGuard {
                    key: self.key.clone(),
                    map: self.map.clone(),
                };
                async {
                    match init(guard).await {
                        Ok(v) => Ok(Arc::new(v)),
                        Err(e) => {
                            // clean up immediately on failure so a future request can retry
                            map.remove(&key);
                            Err(e)
                        }
                    }
                }
            })
            .await?
            .clone();

        if did_initialize {
            Ok((value, InFlightRole::Leader))
        } else {
            Ok((value, InFlightRole::Joiner))
        }
    }
}

/// Removes the entry from the inflight map when dropped.
///
/// For queries, drop this immediately after `get_or_try_init` returns so subsequent requests
/// are not deduplicated against a completed response.
/// For subscriptions, move this into the upstream pump task so the entry remains in the map
/// (and joiners can find it) for the full lifetime of the stream.
pub struct InFlightCleanupGuard<K, V, S = ABuildHasher>
where
    K: Eq + Hash,
    S: BuildHasher + Clone,
{
    key: K,
    map: InFlightMap<K, V, S>,
}

impl<K, V, S> Drop for InFlightCleanupGuard<K, V, S>
where
    K: Eq + Hash,
    S: BuildHasher + Clone,
{
    fn drop(&mut self) {
        self.map.remove(&self.key);
    }
}
