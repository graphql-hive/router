use std::collections::{BTreeMap, HashMap, HashSet};

use hive_router_config::override_labels::{LabelOverrideValue, OverrideLabelsConfig};
use hive_router_internal::expressions::CompileExpression;
use hive_router_plan_executor::execution::client_request_details::ClientRequestDetails;
use hive_router_query_planner::{
    graph::{PlannerOverrideContext, PERCENTAGE_SCALE_FACTOR},
    state::supergraph_state::SupergraphState,
};
use rand::RngExt;
use vrl::{
    compiler::Program as VrlProgram,
    compiler::TargetValue as VrlTargetValue,
    core::Value as VrlValue,
    prelude::{
        state::RuntimeState as VrlState, Context as VrlContext, ExpressionError,
        TimeZone as VrlTimeZone,
    },
    value::Secrets as VrlSecrets,
};

#[derive(thiserror::Error, Debug)]
#[error("Failed to compile override label expression for label '{label}': {error}")]
pub struct OverrideLabelsCompileError {
    pub label: String,
    pub error: String,
}

#[derive(thiserror::Error, Debug)]
pub enum LabelEvaluationError {
    #[error(
        "Failed to resolve VRL expression for override label '{label}'. Runtime error: {source}"
    )]
    ExpressionResolutionFailure {
        label: String,
        source: ExpressionError,
    },
    #[error(
        "VRL expression for override label '{label}' did not evaluate to a boolean. Got: {got}"
    )]
    ExpressionWrongType { label: String, got: String },
}

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
pub fn request_override_context<'exec>(
    override_labels_evaluator: &OverrideLabelsEvaluator,
    client_request_details: &ClientRequestDetails<'exec>,
) -> Result<RequestOverrideContext, LabelEvaluationError> {
    let active_flags = override_labels_evaluator.evaluate(client_request_details)?;

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

/// Evaluator for override labels based on configuration.
/// This struct compiles and evaluates the override label expressions.
/// It's intended to be used as a shared state in the router.
pub struct OverrideLabelsEvaluator {
    static_enabled_labels: HashSet<String>,
    expressions: HashMap<String, VrlProgram>,
}

impl OverrideLabelsEvaluator {
    pub(crate) fn from_config(
        override_labels_config: &OverrideLabelsConfig,
    ) -> Result<Self, OverrideLabelsCompileError> {
        let mut static_enabled_labels = HashSet::new();
        let mut expressions = HashMap::new();

        for (label, value) in override_labels_config.iter() {
            match value {
                LabelOverrideValue::Boolean(true) => {
                    static_enabled_labels.insert(label.clone());
                }
                LabelOverrideValue::Expression { expression } => {
                    let program = expression.compile_expression(None).map_err(|err| {
                        OverrideLabelsCompileError {
                            label: label.clone(),
                            error: err.to_string(),
                        }
                    })?;
                    expressions.insert(label.clone(), program);
                }
                _ => {} // Skip false booleans
            }
        }

        Ok(Self {
            static_enabled_labels,
            expressions,
        })
    }

    pub(crate) fn evaluate<'exec>(
        &self,
        client_request: &ClientRequestDetails<'exec>,
    ) -> Result<HashSet<String>, LabelEvaluationError> {
        let mut active_flags = self.static_enabled_labels.clone();

        if self.expressions.is_empty() {
            return Ok(active_flags);
        }

        let mut target = VrlTargetValue {
            value: VrlValue::Object(BTreeMap::from([("request".into(), client_request.into())])),
            metadata: VrlValue::Object(BTreeMap::new()),
            secrets: VrlSecrets::default(),
        };

        let mut state = VrlState::default();
        let timezone = VrlTimeZone::default();
        let mut ctx = VrlContext::new(&mut target, &mut state, &timezone);

        for (label, expression) in &self.expressions {
            match expression.resolve(&mut ctx) {
                Ok(evaluated_value) => match evaluated_value {
                    VrlValue::Boolean(true) => {
                        active_flags.insert(label.clone());
                    }
                    VrlValue::Boolean(false) => {
                        // Do nothing for false
                    }
                    invalid_value => {
                        return Err(LabelEvaluationError::ExpressionWrongType {
                            label: label.clone(),
                            got: format!("{:?}", invalid_value),
                        });
                    }
                },
                Err(err) => {
                    return Err(LabelEvaluationError::ExpressionResolutionFailure {
                        label: label.clone(),
                        source: err,
                    });
                }
            }
        }

        Ok(active_flags)
    }
}
