use std::collections::{BTreeMap, HashMap, HashSet};

use hive_router_config::override_labels::{LabelOverrideValue, OverrideLabelsConfig};
use hive_router_plan_executor::execution::plan::ClientRequestDetails;
use hive_router_query_planner::{
    graph::{PlannerOverrideContext, PERCENTAGE_SCALE_FACTOR},
    state::supergraph_state::SupergraphState,
};
use rand::Rng;
use vrl::{
    compiler::compile as vrl_compile,
    compiler::Program as VrlProgram,
    compiler::TargetValue as VrlTargetValue,
    core::Value as VrlValue,
    prelude::{state::RuntimeState as VrlState, Context as VrlContext, TimeZone as VrlTimeZone},
    stdlib::all as vrl_build_functions,
    value::Secrets as VrlSecrets,
};

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
pub fn request_override_context<'req, F>(
    override_labels_evaluator: &OverrideLabelsEvaluator,
    get_client_request: F,
) -> Result<RequestOverrideContext, PipelineError>
where
    F: FnOnce() -> ClientRequestDetails<'req>,
{
    // No active flags by default - until we implement it
    let active_flags = override_labels_evaluator.evaluate(get_client_request);

    // for (flag_name, override_value) in override_labels_config.iter() {
    //     match override_value {
    //         LabelOverrideValue::Boolean(true) => {
    //             active_flags.insert(flag_name.clone());
    //         }
    //         // For other cases, we currently do nothing
    //         _ => {
    //             // TODO: support expressions
    //         }
    //     }
    // }

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
    pub(crate) fn from_config(override_labels_config: &OverrideLabelsConfig) -> Self {
        let mut static_enabled_labels = HashSet::new();
        let mut expressions = HashMap::new();
        let vrl_functions = vrl_build_functions();

        for (label, value) in override_labels_config.iter() {
            match value {
                LabelOverrideValue::Boolean(true) => {
                    static_enabled_labels.insert(label.clone());
                }
                LabelOverrideValue::Expression { expression } => {
                    let compilation_result = vrl_compile(expression, &vrl_functions).unwrap();
                    expressions.insert(label.clone(), compilation_result.program);
                }
                _ => {} // Skip false booleans
            }
        }

        Self {
            static_enabled_labels,
            expressions,
        }
    }

    pub fn evaluate<'req, F>(&self, get_client_request: F) -> HashSet<String>
    where
        F: FnOnce() -> ClientRequestDetails<'req>,
    {
        let mut active_flags = self.static_enabled_labels.clone();

        if self.expressions.is_empty() {
            return active_flags;
        }

        let client_request = get_client_request();
        let mut target = VrlTargetValue {
            value: VrlValue::Object(BTreeMap::from([(
                "request".into(),
                (&client_request).into(),
            )])),
            metadata: VrlValue::Object(BTreeMap::new()),
            secrets: VrlSecrets::default(),
        };

        let mut state = VrlState::default();
        let timezone = VrlTimeZone::default();
        let mut ctx = VrlContext::new(&mut target, &mut state, &timezone);

        for (label, expression) in &self.expressions {
            let evaluated_value = expression.resolve(&mut ctx).unwrap();

            match evaluated_value {
                VrlValue::Boolean(true) => {
                    active_flags.insert(label.clone());
                }
                VrlValue::Boolean(false) => {
                    // Do nothing for false
                }
                _ => {
                    // TODO: error handling for non-boolean results
                }
            }
        }

        active_flags
    }
}
