use std::sync::Arc;

use graphql_tools::validation::utils::ValidationError;
use hive_router_internal::telemetry::TelemetryContext;
use hive_router_query_planner::planner::plan_nodes::QueryPlan;
use moka::future::Cache;
use moka::Entry;

use crate::pipeline::normalize::GraphQLNormalizationPayload;
use crate::pipeline::parser::ParseCacheEntry;

#[derive(Clone)]
pub struct CacheState {
    pub parse_cache: Cache<u64, ParseCacheEntry>,
    pub validate_cache: Cache<u64, Arc<Vec<ValidationError>>>,
    pub normalize_cache: Cache<u64, Arc<GraphQLNormalizationPayload>>,
    pub plan_cache: Cache<u64, Arc<QueryPlan>>,
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
            validate_cache: Cache::new(1000),
            normalize_cache: Cache::new(1000),
            plan_cache: Cache::new(1000),
        }
    }

    pub fn on_schema_change(&self) {
        self.plan_cache.invalidate_all();
        self.validate_cache.invalidate_all();
        self.normalize_cache.invalidate_all();
    }
}

pub fn register_cache_size_observers(
    telemetry_context: Arc<TelemetryContext>,
    cache_state: Arc<CacheState>,
) {
    let metrics = &telemetry_context.metrics.cache;

    let parse_cache = Arc::clone(&cache_state);
    metrics
        .parse
        .observe_size_with(move || parse_cache.parse_cache.entry_count());

    let normalize_cache = Arc::clone(&cache_state);
    metrics
        .normalize
        .observe_size_with(move || normalize_cache.normalize_cache.entry_count());

    let validate_cache = Arc::clone(&cache_state);
    metrics
        .validate
        .observe_size_with(move || validate_cache.validate_cache.entry_count());

    let plan_cache = Arc::clone(&cache_state);
    metrics
        .plan
        .observe_size_with(move || plan_cache.plan_cache.entry_count());
}
