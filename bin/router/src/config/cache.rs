use human_size::Size;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// What bounds a single in-memory cache.
///
/// One or the other, never both: moka keeps a single capacity dimension, so a cache is
/// bounded either by how many entries it holds or by how many bytes they add up to.
#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, PartialEq)]
#[serde(untagged, deny_unknown_fields)]
#[non_exhaustive]
pub enum CacheLimitsConfig {
    /// Bounded by entry count. Older entries are evicted once the cache is full.
    Entries {
        #[serde(default = "default_max_entries")]
        max_entries: u64,
    },
    /// Bounded by the estimated heap the cached values hold on to.
    ///
    /// Written with a unit - `256MB`, `512MiB`, `64KiB` - by the same parser as
    /// `limits.max_request_body_size`. Mind that `KB` there means 1024 bytes while `kB` and
    /// `MB` are the SI 1000 and 1000000, so spell out `KiB`/`MiB`/`GiB` when you mean powers
    /// of two. A bare number is rejected; give it a unit.
    ///
    /// The estimate walks each value as it is inserted, so it follows the real shape of
    /// a document or a query plan rather than the length of the query that produced it.
    /// It counts the key, the value, and a fixed allowance for moka's own per-entry
    /// bookkeeping. A value reached through a shared `Arc` is counted by every cache
    /// that points at it, so the total errs high rather than low.
    Size {
        #[schemars(with = "String")]
        max_size: Size,
    },
}

impl Default for CacheLimitsConfig {
    fn default() -> Self {
        CacheLimitsConfig::Entries {
            max_entries: default_max_entries(),
        }
    }
}

impl CacheLimitsConfig {
    /// Bounds the cache by entry count.
    pub fn with_max_entries(max_entries: u64) -> Self {
        CacheLimitsConfig::Entries { max_entries }
    }

    /// Bounds the cache by the estimated heap of its entries.
    pub fn with_max_size(max_size: Size) -> Self {
        CacheLimitsConfig::Size { max_size }
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
/// Call [`Self::set_max_entries`] or [`Self::set_max_size`] to bring custom limits for
/// this variant instead.
///
/// The storage is private on purpose: adding a new limit later only adds a new `set_*`
/// method here, and existing plugin code keeps compiling. An override replaces the whole
/// [`CacheLimitsConfig`] for that cache, which is also what keeps entries and bytes from
/// being set at the same time.
#[derive(Debug, Default, Clone)]
pub struct CacheOverride {
    inner: Option<CacheLimitsConfig>,
}

impl CacheOverride {
    /// Bounds this variant's cache by entry count instead of the router config's limit.
    /// `0` turns the cache off.
    /// Returns the mutable reference so further `set_*` calls can be chained.
    pub fn set_max_entries(&mut self, max_entries: u64) -> &mut Self {
        self.inner = Some(CacheLimitsConfig::with_max_entries(max_entries));
        self
    }

    /// Bounds this variant's cache by estimated heap instead of the router config's limit.
    /// `0` turns the cache off.
    pub fn set_max_size(&mut self, max_size: Size) -> &mut Self {
        self.inner = Some(CacheLimitsConfig::with_max_size(max_size));
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
        assert_eq!(
            from_empty.router.parsing,
            CacheLimitsConfig::with_max_entries(1000)
        );
        assert_eq!(
            from_empty.supergraph.query_plans,
            CacheLimitsConfig::with_max_entries(1000)
        );
    }

    #[test]
    fn should_keep_defaults_for_unset_siblings() {
        let config: CacheConfig =
            serde_json::from_str(r#"{"supergraph": {"query_plans": {"max_entries": 5}}}"#).unwrap();

        assert_eq!(
            config.supergraph.query_plans,
            CacheLimitsConfig::with_max_entries(5)
        );
        assert_eq!(
            config.supergraph.validation,
            CacheLimitsConfig::with_max_entries(1000)
        );
        assert_eq!(
            config.router.parsing,
            CacheLimitsConfig::with_max_entries(1000)
        );
    }

    #[test]
    fn should_read_the_units_we_document() {
        let bytes = |text: &str| {
            text.parse::<Size>()
                .unwrap_or_else(|err| panic!("{text}: {err}"))
                .to_bytes()
        };

        assert_eq!(bytes("512B"), 512);
        assert_eq!(bytes("256MB"), 256_000_000, "MB is the SI 1000000");
        assert_eq!(bytes("1GiB"), 1 << 30);
        assert_eq!(bytes("1kB"), 1_000);
        assert_eq!(
            bytes("1KB"),
            1_024,
            "KB is 1024 here, unlike kB - documented so nobody has to find out the hard way"
        );
        assert!(
            "1024".parse::<Size>().is_err(),
            "a bare number has no unit, so it is rejected rather than guessed at"
        );
    }

    #[test]
    fn should_read_a_byte_budget() {
        let config: CacheConfig =
            serde_json::from_str(r#"{"supergraph": {"query_plans": {"max_size": "256MB"}}}"#)
                .unwrap();

        assert_eq!(
            config.supergraph.query_plans,
            CacheLimitsConfig::with_max_size("256MB".parse().unwrap())
        );
    }

    #[tokio::test]
    async fn should_evict_once_the_byte_budget_is_spent() {
        use crate::cache_state::build_cache;

        // every entry is charged the per-entry allowance on top of its value, so a budget of
        // four allowances holds roughly four entries and not the ten we push through it
        let budget = 4 * crate::heap_size::entry_weight::<u64, String>(&String::new()) as u64;
        let cache: moka::future::Cache<u64, String> = build_cache(
            &CacheLimitsConfig::with_max_size(format!("{budget}B").parse().unwrap()),
        );

        for key in 0..10u64 {
            cache.insert(key, String::new()).await;
        }
        cache.run_pending_tasks().await;

        assert!(
            cache.entry_count() <= 5,
            "a byte budget has to evict: {} entries survived {budget} bytes",
            cache.entry_count()
        );
        assert!(
            cache.weighted_size() <= budget,
            "the cache kept {} bytes of a {budget} byte budget",
            cache.weighted_size()
        );
    }

    #[test]
    fn should_refuse_both_limits_on_one_cache() {
        // moka bounds a cache one way or the other, so asking for both is a config mistake
        let both = serde_json::from_str::<CacheConfig>(
            r#"{"router": {"parsing": {"max_entries": 5, "max_size": "1MB"}}}"#,
        );

        assert!(both.is_err(), "expected an error, got {both:?}");
    }
}
