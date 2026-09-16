use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Limits for a single in-memory cache (router config file).
///
/// Plugin authors don't build this directly - see [`CacheOverride`] and
/// `SupergraphOptions::cache` for the per-variant plugin API.
#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct CacheLimitsConfig {
    /// The maximum number of entries to keep. Older entries are evicted once the cache is full.
    #[serde(default = "default_max_entries")]
    pub max_entries: u64,
}

impl Default for CacheLimitsConfig {
    fn default() -> Self {
        Self {
            max_entries: default_max_entries(),
        }
    }
}

impl CacheLimitsConfig {
    /// Sets the maximum number of entries
    pub fn with_max_entries(mut self, max_entries: u64) -> Self {
        self.max_entries = max_entries;
        self
    }
}

fn default_max_entries() -> u64 {
    1000
}

/// Limits for the caches the router keeps while processing an operation.
#[derive(Debug, Default, Deserialize, Serialize, JsonSchema, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct CacheConfig {
    /// Caches with a single instance per router process.
    #[serde(default)]
    pub router: RouterCacheConfig,

    /// Caches with one instance per supergraph the router serves.
    ///
    /// A plugin serving extra supergraph variants gets its own copy of each of these,
    /// so the memory here is multiplied by the number of variants that are alive.
    /// Plugins can override each limit per variant.
    #[serde(default)]
    pub supergraph: SupergraphCacheConfig,
}

/// Caches with a single instance per router process.
#[derive(Debug, Default, Deserialize, Serialize, JsonSchema, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct RouterCacheConfig {
    /// Parsed GraphQL documents, keyed by the incoming query string.
    #[serde(default)]
    pub parsing: CacheLimitsConfig,
}

/// Caches with one instance per supergraph the router serves.
#[derive(Debug, Default, Deserialize, Serialize, JsonSchema, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct SupergraphCacheConfig {
    /// Validation results for an operation against this supergraph.
    #[serde(default)]
    pub validation: CacheLimitsConfig,
    /// Normalized operations, ready to be planned.
    #[serde(default)]
    pub normalization: CacheLimitsConfig,
    /// Query plans built for this supergraph.
    #[serde(default)]
    pub query_plans: CacheLimitsConfig,
}

/// Per-variant overrides for [`SupergraphCacheConfig`], set by plugins that serve their own
/// supergraph variants.
///
/// `SupergraphOptions::default()` already carries a defaulted instance of this, so plugins
/// only touch the caches they care about - everything left alone inherits the router
/// config's `cache.supergraph` value:
///
/// ```
/// # use hive_router::plugins::hooks::on_supergraph_load::SupergraphOptions;
/// let mut options = SupergraphOptions::default();
/// options.cache.query_plans.set_max_entries(100);
/// ```
#[derive(Debug, Default, Clone)]
#[non_exhaustive]
pub struct SupergraphCacheOverrides {
    /// Override for this variant's validation cache.
    pub validation: CacheOverride,
    /// Override for this variant's normalization cache.
    pub normalization: CacheOverride,
    /// Override for this variant's query plan cache.
    pub query_plans: CacheOverride,
}

/// Override for a single per-supergraph cache.
///
/// Starts out inheriting the router config's `cache.supergraph` value,
/// so a plugin that doesn't care about caches picks up whatever the operator configured.
/// Call [`Self::set_max_entries`] to bring custom limits for this variant instead.
///
/// The storage is private on purpose: adding a new limit dimension later only adds a new
/// `set_*` method here, and existing plugin code keeps compiling. An override replaces the
/// whole [`CacheLimitsConfig`] for that cache rather than one field of it.
#[derive(Debug, Default, Clone)]
pub struct CacheOverride {
    inner: Option<CacheLimitsConfig>,
}

impl CacheOverride {
    /// Uses `max_entries` for this variant instead of the router config's value.
    /// `0` turns the cache off. Returns the mutable reference so further
    /// `set_*` calls can be chained once more dimensions exist.
    pub fn set_max_entries(&mut self, max_entries: u64) -> &mut Self {
        self.inner = Some(CacheLimitsConfig {
            max_entries,
            ..Default::default()
        });
        self
    }

    /// Turns the cache off for this variant. Shorthand for `set_max_entries(0)`.
    pub fn disable(&mut self) -> &mut Self {
        self.set_max_entries(0)
    }

    /// Goes back to inheriting the router config's value for this cache.
    pub fn inherit(&mut self) -> &mut Self {
        self.inner = None;
        self
    }

    /// The limits to actually build the cache with, given what the router config asked for.
    pub(crate) fn resolve<'a>(&'a self, inherited: &'a CacheLimitsConfig) -> &'a CacheLimitsConfig {
        self.inner.as_ref().unwrap_or(inherited)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_default_every_cache_when_config_is_empty() {
        // guards against drift between the `serde(default)` fns and the `Default` impls
        let from_empty: CacheConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(from_empty, CacheConfig::default());
        assert_eq!(from_empty.router.parsing.max_entries, 1000);
        assert_eq!(from_empty.supergraph.query_plans.max_entries, 1000);
    }

    #[test]
    fn should_keep_defaults_for_unset_siblings() {
        let config: CacheConfig =
            serde_json::from_str(r#"{"supergraph": {"query_plans": {"max_entries": 5}}}"#).unwrap();

        assert_eq!(config.supergraph.query_plans.max_entries, 5);
        assert_eq!(config.supergraph.validation.max_entries, 1000);
        assert_eq!(config.router.parsing.max_entries, 1000);
    }

    #[tokio::test]
    async fn should_cache_nothing_when_max_entries_is_zero() {
        // `0` is documented as "off", so make sure moka actually honors it
        let cache: moka::future::Cache<u64, u64> = moka::future::Cache::new(0);
        cache.insert(1, 1).await;
        cache.run_pending_tasks().await;

        assert_eq!(cache.entry_count(), 0);
    }
}
