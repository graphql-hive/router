use std::hash::Hash;
use std::sync::Arc;

use crate::config::cache::CacheLimitsConfig;
use crate::telemetry::TelemetryContext;
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

pub(crate) fn build_cache<K, V>(limits: &CacheLimitsConfig) -> moka::future::Cache<K, V>
where
    K: Hash + Eq + Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
{
    let mut builder = moka::future::Cache::builder().max_capacity(limits.max_entries);
    if let Some(ttl) = limits.time_to_live.filter(|ttl| !ttl.is_zero()) {
        builder = builder.time_to_live(ttl);
    }
    if let Some(tti) = limits.time_to_idle.filter(|tti| !tti.is_zero()) {
        builder = builder.time_to_idle(tti);
    }
    builder.build()
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn should_build_cache_with_combined_ttl_and_tti() {
        let limits = CacheLimitsConfig::default()
            .with_max_entries(10)
            .with_time_to_live(Some(Duration::from_secs(1800)))
            .with_time_to_idle(Some(Duration::from_secs(300)));
        let cache: moka::future::Cache<u64, u64> = build_cache(&limits);

        assert_eq!(cache.policy().max_capacity(), Some(10));
        assert_eq!(
            cache.policy().time_to_live(),
            Some(Duration::from_secs(1800))
        );
        assert_eq!(
            cache.policy().time_to_idle(),
            Some(Duration::from_secs(300))
        );
    }

    #[test]
    fn should_build_cache_without_expiry_by_default() {
        let cache: moka::future::Cache<u64, u64> = build_cache(&CacheLimitsConfig::default());

        assert_eq!(cache.policy().max_capacity(), Some(1000));
        assert_eq!(cache.policy().time_to_live(), None);
        assert_eq!(cache.policy().time_to_idle(), None);
    }

    #[test]
    fn should_ignore_zero_ttl_and_tti() {
        // moka would expire entries immediately on a zero duration; treat it as unset instead
        let limits = CacheLimitsConfig::default()
            .with_time_to_live(Some(Duration::ZERO))
            .with_time_to_idle(Some(Duration::ZERO));
        let cache: moka::future::Cache<u64, u64> = build_cache(&limits);

        assert_eq!(cache.policy().time_to_live(), None);
        assert_eq!(cache.policy().time_to_idle(), None);
    }

    #[tokio::test]
    async fn should_cache_nothing_when_max_entries_is_zero() {
        let cache: moka::future::Cache<u64, u64> =
            build_cache(&CacheLimitsConfig::default().with_max_entries(0));
        cache.insert(1, 1).await;
        cache.run_pending_tasks().await;

        assert_eq!(cache.entry_count(), 0);
    }

    #[tokio::test]
    async fn should_expire_entries_after_ttl() {
        let cache: moka::future::Cache<u64, u64> = build_cache(
            &CacheLimitsConfig::default().with_time_to_live(Some(Duration::from_millis(20))),
        );
        cache.insert(1, 1).await;
        cache.run_pending_tasks().await;
        assert_eq!(cache.entry_count(), 1);

        tokio::time::sleep(Duration::from_millis(50)).await;
        cache.run_pending_tasks().await;

        assert!(cache.get(&1).await.is_none());
    }
}
