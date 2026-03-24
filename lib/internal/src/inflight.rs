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
    #[inline]
    pub async fn get_or_try_init<E, F, Fut>(self, init: F) -> Result<(Arc<V>, InFlightRole), E>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<V, E>>,
    {
        let mut did_initialize = false;
        let key = self.key.clone();
        let map = self.map.clone();

        let value = self
            .cell
            .get_or_try_init(|| {
                did_initialize = true;
                async {
                    let _cleanup = InFlightCleanupGuard { key, map };
                    init().await.map(Arc::new)
                }
            })
            .await?
            .clone();

        let role = if did_initialize {
            InFlightRole::Leader
        } else {
            InFlightRole::Joiner
        };

        Ok((value, role))
    }
}

struct InFlightCleanupGuard<K, V, S = ABuildHasher>
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
        // It's important to remove the entry from the map before returning the result.
        // This ensures that once the OnceCell is set, no future requests can join it.
        // The cache is for the lifetime of the in-flight request only.
        self.map.remove(&self.key);
    }
}
