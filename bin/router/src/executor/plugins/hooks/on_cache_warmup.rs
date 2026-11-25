use std::sync::Arc;

use crate::executor::hooks::on_supergraph_load::SupergraphSnapshot;

pub use crate::telemetry::metrics::catalog::values::CacheWarmupSource;

/// Runs once per plan cache warmup, after the router collected operations from its own sources
/// and before any of them gets planned. Only runs when `cache.warmup.enabled` is on.
pub struct OnCacheWarmupHookPayload<'exec> {
    /// The supergraph being warmed up. The router switches to it once warmup is done.
    pub supergraph: &'exec SupergraphSnapshot,
    /// Operations to plan, first to last. Add, drop or reorder them as you like.
    /// If `cache.warmup.timeout` runs out, whatever is left at the end doesn't get planned.
    pub operations: Vec<CacheWarmupOperation>,
}

/// One operation to plan during cache warmup.
#[derive(Clone, Debug)]
pub struct CacheWarmupOperation {
    pub(crate) query: Arc<String>,
    pub(crate) operation_name: Option<String>,
    pub(crate) source: CacheWarmupSource,
}

impl CacheWarmupOperation {
    /// An operation a plugin wants planned.
    /// Pass the operation name your clients send, if they send one.
    pub fn new(query: impl Into<String>, operation_name: Option<String>) -> Self {
        Self::from_text(
            Arc::new(query.into()),
            operation_name,
            CacheWarmupSource::Plugin,
        )
    }

    pub(crate) fn from_text(
        query: Arc<String>,
        operation_name: Option<String>,
        source: CacheWarmupSource,
    ) -> Self {
        Self {
            query,
            operation_name,
            source,
        }
    }

    pub fn operation_name(&self) -> Option<&str> {
        self.operation_name.as_deref()
    }

    /// Where the operation came from.
    pub fn source(&self) -> CacheWarmupSource {
        self.source
    }
}
