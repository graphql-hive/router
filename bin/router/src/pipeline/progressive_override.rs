use std::collections::{BTreeMap, HashSet};

use query_planner::{
    graph::{PlannerOverrideContext, PERCENTAGE_SCALE_FACTOR},
    state::supergraph_state::SupergraphState,
};
use rand::Rng;

use super::error::PipelineError;

/// Contains the request-specific context for progressive overrides.
/// This is stored in the request extensions
#[derive(Debug, Clone)]
pub struct RequestOverrideContext {
    /// The set of override flags that are active for this request.
    pub active_flags: HashSet<String>,
    /// The randomly generated percentage value for this request.
    pub percentage_value: u64,
}

#[inline]
pub fn request_override_context() -> Result<RequestOverrideContext, PipelineError> {
    // No active flags by default - until we implement it
    let active_flags = HashSet::new();

    // Generate the random percentage value for this request.
    // Percentage is 0 - 100_000_000_000 (100*PERCENTAGE_SCALE_FACTOR)
    // 0 = 0%
    // 100_000_000_000 = 100%
    // 50_000_000_000 = 50%
    // 50_123_456_789 = 50.12345678%
    let percentage_value: u64 = rand::rng().random_range(0..=(100 * PERCENTAGE_SCALE_FACTOR));

    let override_context = RequestOverrideContext {
        active_flags,
        percentage_value,
    };

    Ok(override_context)
}

impl From<&RequestOverrideContext> for PlannerOverrideContext {
    fn from(value: &RequestOverrideContext) -> Self {
        Self::new(value.active_flags.clone(), value.percentage_value)
    }
}

/// Deterministic context representing the outcome of progressive override rules.
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct StableOverrideContext {
    /// Stores the active status of only the flags relevant to the supergraph.
    active_flags: BTreeMap<String, bool>,
    /// Stores the boolean outcome of the percentage check for each relevant threshold.
    percentage_outcomes: BTreeMap<u64, bool>,
}

impl StableOverrideContext {
    pub fn new(
        supergraph: &SupergraphState,
        request_override_context: &RequestOverrideContext,
    ) -> Self {
        let mut active_flags = BTreeMap::new();
        for flag_name in &supergraph.progressive_overrides.flags {
            let is_active = request_override_context.active_flags.contains(flag_name);
            active_flags.insert(flag_name.clone(), is_active);
        }

        let mut percentage_outcomes = BTreeMap::new();
        for &threshold in &supergraph.progressive_overrides.percentages {
            let in_range = request_override_context.percentage_value < threshold;
            percentage_outcomes.insert(threshold, in_range);
        }

        StableOverrideContext {
            active_flags,
            percentage_outcomes,
        }
    }
}
