use std::sync::Arc;

use hive_router_internal::telemetry::TelemetryContext;
use moka::Entry;

use crate::schema_state::SchemaState;
use crate::shared_state::RouterSharedState;

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

    metrics
        .parse
        .observe_size_with(move || shared_state.parse_cache.entry_count());

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

    let normalize_schema_state = Arc::clone(&schema_state);
    metrics.normalize.observe_size_with(move || {
        let mut total = 0;
        normalize_schema_state
            .for_each_runtime(|runtime| total += runtime.normalize_cache.entry_count());
        total
    });

    metrics.plan.observe_size_with(move || {
        let mut total = 0;
        schema_state.for_each_runtime(|runtime| total += runtime.plan_cache.entry_count());
        total
    });
}
