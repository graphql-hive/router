use hashbrown::HashSet;

use axum::body::Body;
use http::Request;
use query_planner::graph::{PlannerOverrideContext, PERCENTAGE_SCALE_FACTOR};
use rand::Rng;

use super::error::PipelineError;
use super::gateway_layer::{GatewayPipelineLayer, GatewayPipelineStepDecision, ProcessorLayer};

/// Contains the request-specific context for progressive overrides.
/// This is stored in the request extensions
#[derive(Debug, Clone)]
pub struct RequestOverrideContext {
    /// The set of override flags that are active for this request.
    pub active_flags: HashSet<String>,
    /// The randomly generated percentage value for this request.
    pub percentage_value: u64,
}

#[derive(Clone, Debug, Default)]
pub struct ProgressiveOverrideExtractor;

impl ProgressiveOverrideExtractor {
    pub fn new_layer() -> ProcessorLayer<Self> {
        ProcessorLayer::new(Self)
    }
}

#[async_trait::async_trait]
impl GatewayPipelineLayer for ProgressiveOverrideExtractor {
    #[tracing::instrument(level = "trace", name = "ProgressiveOverrideExtractor", skip_all)]
    async fn process(
        &self,
        mut req: Request<Body>,
    ) -> Result<(Request<Body>, GatewayPipelineStepDecision), PipelineError> {
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

        req.extensions_mut().insert(override_context);

        Ok((req, GatewayPipelineStepDecision::Continue))
    }
}

impl From<&RequestOverrideContext> for PlannerOverrideContext {
    fn from(value: &RequestOverrideContext) -> Self {
        Self::new(value.active_flags.clone(), value.percentage_value)
    }
}
