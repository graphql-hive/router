use std::hash::Hash;
use std::sync::Arc;

use crate::config::cache::CacheLimitsConfig;
use crate::heap_size::{entry_weight, HeapSize};
use crate::telemetry::TelemetryContext;
use moka::future::Cache;
use moka::Entry;

use crate::schema_state::SchemaState;
use crate::shared_state::RouterSharedState;

/// Builds a cache bounded the way the config asks for: by entry count, or by the heap its
/// entries are estimated to hold on to. moka carries a single capacity dimension, so the
/// weigher is only installed for the byte-budgeted case - with one attached, `max_capacity`
/// would be a weight and the entry count would stop meaning anything.
pub fn build_cache<K, V>(limits: &CacheLimitsConfig) -> Cache<K, V>
where
    K: Hash + Eq + Send + Sync + 'static,
    V: Clone + Send + Sync + HeapSize + 'static,
{
    match limits {
        CacheLimitsConfig::Entries { max_entries } => Cache::new(*max_entries),
        CacheLimitsConfig::Size { max_size } => Cache::builder()
            .weigher(|_key: &K, value: &V| entry_weight::<K, V>(value))
            .max_capacity(max_size.to_bytes())
            .build(),
    }
}

#[derive(Clone, Copy, Debug)]
pub enum CacheHitMiss {
    Hit,
    Miss,
    Error,
}

impl CacheHitMiss {}

pub trait EntryResultHitMissExt<V, E> {
    fn into_result_with_hit_miss(self, on_hit_miss: impl FnOnce(CacheHitMiss)) -> Result<V, E>;
}

pub trait EntryValueHitMissExt<V> {
    fn into_value_with_hit_miss(self, on_hit_miss: impl FnOnce(CacheHitMiss)) -> V;
}

impl<K, V, E> EntryResultHitMissExt<V, E> for Result<Entry<K, V>, E> {
    fn into_result_with_hit_miss(self, on_hit_miss: impl FnOnce(CacheHitMiss)) -> Result<V, E> {
        match self {
            Ok(entry) => {
                let hit_miss = if entry.is_fresh() {
                    CacheHitMiss::Miss
                } else {
                    CacheHitMiss::Hit
                };
                on_hit_miss(hit_miss);
                Ok(entry.into_value())
            }
            Err(err) => {
                on_hit_miss(CacheHitMiss::Error);
                Err(err)
            }
        }
    }
}

impl<K, V> EntryValueHitMissExt<V> for Entry<K, V> {
    fn into_value_with_hit_miss(self, on_hit_miss: impl FnOnce(CacheHitMiss)) -> V {
        let hit_miss = if self.is_fresh() {
            CacheHitMiss::Miss
        } else {
            CacheHitMiss::Hit
        };
        on_hit_miss(hit_miss);
        self.into_value()
    }
}

pub fn register_cache_size_observers(
    telemetry_context: Arc<TelemetryContext>,
    shared_state: Arc<RouterSharedState>,
    schema_state: Arc<SchemaState>,
) {
    let metrics = &telemetry_context.metrics.cache;

    let parse_cache = shared_state.parse_cache.clone();
    metrics
        .parse
        .observe_size_with(move || shared_state.parse_cache.entry_count());
    metrics
        .parse
        .observe_size_bytes_with(move || parse_cache.weighted_size());

    // validate/normalize/plan caches live on `RouterSupergraphRuntime` (one per supergraph variant,
    // dropped with it on retirement) rather than on the shared state, so sum entry counts across
    // every runtime currently alive (the configured default plus any plugin-selected ones still cached)

    let validate_schema_state = Arc::clone(&schema_state);
    metrics.validate.observe_size_with(move || {
        let mut total = 0;
        validate_schema_state
            .for_each_runtime(|runtime| total += runtime.validate_cache.entry_count());
        total
    });

    let validate_bytes_schema_state = Arc::clone(&schema_state);
    metrics.validate.observe_size_bytes_with(move || {
        let mut total = 0;
        validate_bytes_schema_state
            .for_each_runtime(|runtime| total += runtime.validate_cache.weighted_size());
        total
    });

    let normalize_schema_state = Arc::clone(&schema_state);
    metrics.normalize.observe_size_with(move || {
        let mut total = 0;
        normalize_schema_state
            .for_each_runtime(|runtime| total += runtime.normalize_cache.entry_count());
        total
    });

    let normalize_bytes_schema_state = Arc::clone(&schema_state);
    metrics.normalize.observe_size_bytes_with(move || {
        let mut total = 0;
        normalize_bytes_schema_state
            .for_each_runtime(|runtime| total += runtime.normalize_cache.weighted_size());
        total
    });

    let plan_bytes_schema_state = Arc::clone(&schema_state);
    metrics.plan.observe_size_bytes_with(move || {
        let mut total = 0;
        plan_bytes_schema_state
            .for_each_runtime(|runtime| total += runtime.plan_cache.weighted_size());
        total
    });

    metrics.plan.observe_size_with(move || {
        let mut total = 0;
        schema_state.for_each_runtime(|runtime| total += runtime.plan_cache.entry_count());
        total
    });
}
