use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct CacheLimitsConfig {
    /// The maximum number of entries to keep. Older entries are evicted once the cache is full.
    #[serde(default = "default_max_entries")]
    pub max_entries: u64,
    /// Time to live: entries expire this long after being written, even if still hot.
    /// Entry is evicted on whichever of TTL/TTI fires first.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "humantime_serde::deserialize",
        serialize_with = "humantime_serde::serialize"
    )]
    #[schemars(with = "Option<String>")]
    pub time_to_live: Option<Duration>,
    /// Time to idle: entries expire after being unread/unwritten for this long.
    /// Entry is evicted on whichever of TTL/TTI fires first.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "humantime_serde::deserialize",
        serialize_with = "humantime_serde::serialize"
    )]
    #[schemars(with = "Option<String>")]
    pub time_to_idle: Option<Duration>,
}

impl Default for CacheLimitsConfig {
    fn default() -> Self {
        Self {
            max_entries: default_max_entries(),
            time_to_live: None,
            time_to_idle: None,
        }
    }
}

impl CacheLimitsConfig {
    /// Sets the maximum number of entries
    pub fn with_max_entries(mut self, max_entries: u64) -> Self {
        self.max_entries = max_entries;
        self
    }

    /// Sets the time to live (`None` disables TTL expiry)
    pub fn with_time_to_live(mut self, time_to_live: Option<Duration>) -> Self {
        self.time_to_live = time_to_live;
        self
    }

    /// Sets the time to idle (`None` disables idle expiry)
    pub fn with_time_to_idle(mut self, time_to_idle: Option<Duration>) -> Self {
        self.time_to_idle = time_to_idle;
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
    /// Parsed GraphQL documents, keyed by the hash of the incoming query string.
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

/// Per-variant overrides for [`SupergraphCacheConfig`],
/// set by plugins that serve their own supergraph variants.
///
/// `SupergraphOptions::default()` already carries a defaulted instance of this, so plugins
/// only touch the caches they care about - everything left alone inherits the router
/// config's `cache.supergraph` value:
///
/// ```
/// use hive_router::plugins::hooks::on_supergraph_load::SupergraphOptions;
/// use std::time::Duration;
/// let mut options = SupergraphOptions::default();
/// options.cache.query_plans.set_max_entries(100);
/// options
///     .cache
///     .query_plans
///     .set_time_to_idle(Some(Duration::from_secs(300)));
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub(crate) enum CacheSetting<T> {
    #[default]
    Inherit,
    Override(T),
}

impl<T: Copy> CacheSetting<T> {
    fn resolve(&self, inherited: T) -> T {
        match self {
            Self::Inherit => inherited,
            Self::Override(value) => *value,
        }
    }
}

/// Override for a single per-supergraph cache.
///
/// Starts out inheriting the router config's `cache.supergraph` value,
/// so a plugin that doesn't care about caches picks up whatever the operator configured.
/// Call [`Self::set_max_entries`], [`Self::set_time_to_live`] or [`Self::set_time_to_idle`]
/// to bring custom limits for this variant instead. Each dimension inherits independently:
/// dimensions the plugin never touches keep the router config's value, so overriding one
/// never resets the others.
///
/// The fields stay private on purpose: adding a new limit dimension later only adds a new
/// `set_*` method here, and existing plugin code keeps compiling.
#[derive(Debug, Default, Clone)]
pub struct CacheOverride {
    max_entries: CacheSetting<u64>,
    time_to_live: CacheSetting<Option<Duration>>,
    time_to_idle: CacheSetting<Option<Duration>>,
}

impl CacheOverride {
    /// Uses `max_entries` for this variant instead of the router config's value.
    /// `0` turns the cache off.
    /// Returns the mutable reference so further `set_*` calls can be chained.
    pub fn set_max_entries(&mut self, max_entries: u64) -> &mut Self {
        self.max_entries = CacheSetting::Override(max_entries);
        self
    }

    /// Uses `time_to_live` for this variant instead of the router config's value.
    /// Pass `None` to disable TTL expiry for this variant even if the config sets one.
    /// Returns the mutable reference so further `set_*` calls can be chained.
    pub fn set_time_to_live(&mut self, time_to_live: Option<Duration>) -> &mut Self {
        self.time_to_live = CacheSetting::Override(time_to_live);
        self
    }

    /// Uses `time_to_idle` for this variant instead of the router config's value.
    /// Pass `None` to disable idle expiry for this variant even if the config sets one.
    /// Returns the mutable reference so further `set_*` calls can be chained.
    pub fn set_time_to_idle(&mut self, time_to_idle: Option<Duration>) -> &mut Self {
        self.time_to_idle = CacheSetting::Override(time_to_idle);
        self
    }

    /// Turns the cache off for this variant. Shorthand for `set_max_entries(0)`.
    pub fn disable(&mut self) -> &mut Self {
        self.set_max_entries(0)
    }

    /// Goes back to inheriting the router config's value for this cache.
    pub fn inherit(&mut self) -> &mut Self {
        *self = Self::default();
        self
    }

    /// The limits to actually build the cache with, given what the router config asked for.
    pub(crate) fn resolve(&self, inherited: &CacheLimitsConfig) -> CacheLimitsConfig {
        CacheLimitsConfig {
            max_entries: self.max_entries.resolve(inherited.max_entries),
            time_to_live: self.time_to_live.resolve(inherited.time_to_live),
            time_to_idle: self.time_to_idle.resolve(inherited.time_to_idle),
        }
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
        assert_eq!(from_empty.router.parsing.time_to_live, None);
        assert_eq!(from_empty.router.parsing.time_to_idle, None);
        assert_eq!(from_empty.supergraph.query_plans.max_entries, 1000);
        assert_eq!(from_empty.supergraph.query_plans.time_to_live, None);
        assert_eq!(from_empty.supergraph.query_plans.time_to_idle, None);
    }

    #[test]
    fn should_keep_defaults_for_unset_siblings() {
        let config: CacheConfig =
            serde_json::from_str(r#"{"supergraph": {"query_plans": {"max_entries": 5}}}"#).unwrap();

        assert_eq!(config.supergraph.query_plans.max_entries, 5);
        assert_eq!(config.supergraph.query_plans.time_to_live, None);
        assert_eq!(config.supergraph.query_plans.time_to_idle, None);
        assert_eq!(config.supergraph.validation.max_entries, 1000);
        assert_eq!(config.router.parsing.max_entries, 1000);
    }

    #[test]
    fn should_parse_ttl_and_tti() {
        let config: CacheConfig = serde_json::from_str(
            r#"{"supergraph": {"query_plans": {"max_entries": 5, "time_to_live": "30m", "time_to_idle": "5m"}}}"#,
        )
        .unwrap();

        assert_eq!(config.supergraph.query_plans.max_entries, 5);
        assert_eq!(
            config.supergraph.query_plans.time_to_live,
            Some(Duration::from_secs(30 * 60))
        );
        assert_eq!(
            config.supergraph.query_plans.time_to_idle,
            Some(Duration::from_secs(5 * 60))
        );
        // unset siblings stay expiry-free
        assert_eq!(config.supergraph.validation.time_to_live, None);
        assert_eq!(config.supergraph.validation.time_to_idle, None);
    }

    #[test]
    fn should_resolve_each_override_dimension_independently() {
        let inherited = CacheLimitsConfig::default()
            .with_max_entries(9)
            .with_time_to_live(Some(Duration::from_secs(1800)))
            .with_time_to_idle(Some(Duration::from_secs(300)));

        // untouched override inherits everything
        assert_eq!(CacheOverride::default().resolve(&inherited), inherited);

        // overriding one dimension leaves the others inheriting
        let mut ttl_only = CacheOverride::default();
        ttl_only.set_time_to_live(Some(Duration::from_secs(60)));
        let resolved = ttl_only.resolve(&inherited);
        assert_eq!(resolved.max_entries, 9);
        assert_eq!(resolved.time_to_live, Some(Duration::from_secs(60)));
        assert_eq!(resolved.time_to_idle, Some(Duration::from_secs(300)));

        // None disables an inherited expiry without touching the rest
        let mut disabled = CacheOverride::default();
        disabled.set_time_to_live(None).set_time_to_idle(None);
        let resolved = disabled.resolve(&inherited);
        assert_eq!(resolved.max_entries, 9);
        assert_eq!(resolved.time_to_live, None);
        assert_eq!(resolved.time_to_idle, None);
    }
}
