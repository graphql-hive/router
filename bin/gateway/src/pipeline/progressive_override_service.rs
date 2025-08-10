use std::collections::HashSet;

use query_planner::graph::{PlannerOverrideContext, PERCENTAGE_SCALE_FACTOR};
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

impl From<&RequestOverrideContext> for PlannerOverrideContext {
    fn from(value: &RequestOverrideContext) -> Self {
        Self::new(value.active_flags.clone(), value.percentage_value)
    }
}

#[inline]
pub fn progressive_override_extractor() -> Result<RequestOverrideContext, PipelineError> {
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
