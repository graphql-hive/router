use std::sync::Arc;

use hive_router_internal::telemetry::TelemetryContext;
use moka::future::Cache;
use moka::Entry;

use crate::pipeline::parser::ParseCacheEntry;

// schema-dependent caches (validate/normalize/plan/demand-control formula) live on
// `RouterSupergraphRuntime` instead - this only holds the schema-independent parse cache, which
// is safe to share across every supergraph variant.
pub struct CacheState {
    pub parse_cache: Cache<u64, ParseCacheEntry>,
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

impl CacheState {
    pub fn new() -> Self {
        Self {
            parse_cache: Cache::new(1000),
        }
    }
}

pub fn register_cache_size_observers(
    telemetry_context: Arc<TelemetryContext>,
    cache_state: Arc<CacheState>,
) {
    let metrics = &telemetry_context.metrics.cache;

    metrics
        .parse
        .observe_size_with(move || cache_state.parse_cache.entry_count());

    // validate/normalize/plan caches now live on `RouterSupergraphRuntime` (one per supergraph,
    // dropped with it on retirement) instead of one shared `CacheState` cache, so there's no
    // single cache left here to observe the size of.
}
