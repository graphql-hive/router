use std::collections::BTreeMap;
use std::sync::Arc;

use crate::config::demand_control::{
    DemandControlActualCostMode, DemandControlConfig, DemandControlExposeHeadersConfig,
    DemandControlMode,
};
use crate::executor::execution::demand_control::{
    compile_actual_response_shape_cost_plan, compile_actual_subgraph_cost_plan,
    CompiledActualCostPlan, CompiledSubgraphActualCostPlan, DemandControlEvaluation,
    DemandControlExecutionActualCostContext, DemandControlExecutionContext,
    DemandControlExecutionOperationContext, DemandControlExecutionSubgraphsContext,
};
use crate::executor::execution::plan::CoerceVariablesPayload;
use crate::executor::hooks::on_supergraph_load::SupergraphSnapshot;
use crate::query_planner::ast::operation::{OperationDefinition, PlanningFetchOperation};
use crate::query_planner::planner::plan_nodes::{PlanNode, Planning, QueryPlan};
use crate::query_planner::state::supergraph_state::{OperationKind, SupergraphState};
use crate::telemetry::logging::targets;
use crate::telemetry::metrics::demand_control_metrics::DemandControlResultCode;
use crate::telemetry::metrics::Metrics;
use crate::telemetry::traces::spans::graphql::GraphQLSpanOperationIdentity;
use ahash::{HashMap as AHashMap, HashMapExt};
use http::{HeaderName, HeaderValue};
use tracing::{debug, info, warn};

use crate::pipeline::error::{ClientPipelineError, PipelineError};

use super::formula::{
    compile_cost_expr_for_operation, evaluate_formula_plan, DemandControlFormulaPlan,
    FormulaFetchNode, FormulaPlanNode,
};

pub struct DemandControlRuntime {
    config: DemandControlConfig,
    expose_headers_flags: Arc<DemandControlExposeHeadersConfig>,
    metrics: Arc<Metrics>,
}

impl DemandControlRuntime {
    pub fn from_config(
        config: Option<&DemandControlConfig>,
        metrics: Arc<Metrics>,
    ) -> Option<Self> {
        let config = config?;

        if !config.enabled {
            debug!(target: targets::DEMAND_CONTROL, "demand control is disabled");

            return None;
        }

        info!(
            target: targets::DEMAND_CONTROL,
            operation_mode = ?config.operation_cost.mode,
            operation_max_cost = config.operation_cost.max,
            subgraph_budget_mode = ?config.subgraphs_budget.mode,
            default_list_size = ?&config.default_list_size,
            actual_cost_mode = ?config.actual_cost_mode,
            "demand control enabled"
        );

        if config.operation_cost.mode == DemandControlMode::Enforce {
            if config.operation_cost.max == 0 {
                warn!(
                    target: targets::DEMAND_CONTROL,
                    "demand control is in enforce mode with a max cost of 0; all operations with non-zero cost will be rejected"
                );
            }

            if config.default_list_size.all.is_none()
                && config.default_list_size.subgraphs.is_none()
            {
                warn!(
                    target: targets::DEMAND_CONTROL,
                    "demand control is in enforce mode without a default list_size; list fields without an @listSize directive are estimated as 0 and may be under-counted"
                );
            }
        }

        Some(Self {
            expose_headers_flags: Arc::new(config.operation_cost.expose_headers.clone()),
            config: config.clone(),
            metrics,
        })
    }
}

impl DemandControlRuntime {
    pub fn evaluate<'exec>(
        &self,
        supergraph: &'exec SupergraphSnapshot,
        variable_payload: &'exec CoerceVariablesPayload,
        compiled_plan: &'exec DemandControlFormulaPlan,
        operation_identity: GraphQLSpanOperationIdentity<'exec>,
    ) -> Result<DemandControlExecutionContext, PipelineError> {
        let operation_name = operation_identity.name;

        let evaluation = evaluate_formula_plan(
            compiled_plan,
            &supergraph.planner.supergraph,
            variable_payload,
        )?;

        let max_cost = self.config.operation_cost.max;
        let estimated_exceeds_max = evaluation.estimated_cost > max_cost;

        self.metrics.demand_control.record_estimated_cost(
            evaluation.estimated_cost,
            &if estimated_exceeds_max {
                DemandControlResultCode::CostEstimatedTooExpensive
            } else {
                DemandControlResultCode::CostOk
            },
            operation_name,
        );

        if estimated_exceeds_max {
            match self.config.operation_cost.mode {
                DemandControlMode::Enforce => {
                    warn!(
                        target: targets::DEMAND_CONTROL,
                        operation_name = ?operation_name,
                        estimated_cost = evaluation.estimated_cost,
                        max_cost,
                        "rejecting operation: estimated cost exceeds configured max cost"
                    );

                    let mut err_extra_headers: Vec<(HeaderName, HeaderValue)> = vec![];
                    if let Some(header_name) = &self.expose_headers_flags.estimated {
                        err_extra_headers.push((
                            header_name.get_header_ref().to_owned(),
                            evaluation.estimated_cost.into(),
                        ));
                    }

                    if let Some(header_name) = &self.expose_headers_flags.max {
                        err_extra_headers
                            .push((header_name.get_header_ref().to_owned(), max_cost.into()));
                    }

                    return Err(ClientPipelineError::CostEstimatedTooExpensive {
                        response_headers: err_extra_headers,
                    }
                    .into());
                }
                DemandControlMode::Measure => {
                    warn!(
                        target: targets::DEMAND_CONTROL,
                        operation_name = ?operation_name,
                        estimated_cost = evaluation.estimated_cost,
                        max_cost,
                        "measure mode: operation would be rejected in enforce mode"
                    );
                }
            }
        }

        Ok(DemandControlExecutionContext {
            metrics_recorder: self.metrics.demand_control.recorder(),
            actual: DemandControlExecutionActualCostContext {
                cost_mode: self.config.actual_cost_mode,
                cost_plan: compiled_plan.actual_cost_plan.clone(),
            },
            operation: DemandControlExecutionOperationContext {
                operation_max_cost: max_cost,
                expose_headers_flags: self.expose_headers_flags.clone(),
            },
            subgraphs: DemandControlExecutionSubgraphsContext {
                enforcement_mode: self.config.subgraphs_budget.mode,
                blocked_subgraphs: self.list_blocked_subgraphs(&evaluation),
                blocked_subgraphs_enforcement_mode: self.config.subgraphs_budget.mode,
            },
            evaluation,
        })
    }
}

impl DemandControlRuntime {
    fn default_list_size_for_subgraph(&self, subgraph_name: &str) -> usize {
        let default_list_size_cfg = &self.config.default_list_size;

        default_list_size_cfg
            .subgraphs
            .as_ref()
            .and_then(|subgraphs| subgraphs.get(subgraph_name))
            .copied()
            .or(default_list_size_cfg.all)
            .unwrap_or(0)
    }

    /// Returns a list of subgraphs that have exceeded their list size limit, based on static estimation.
    /// This will later be used in order to block subgraphs from being executed, during execution.
    ///
    /// Key is the subgraph name, value is the limit that was exceeded (max).
    #[inline]
    fn list_blocked_subgraphs(
        &self,
        evaluation: &DemandControlEvaluation,
    ) -> BTreeMap<String, u64> {
        let mut over_limit = BTreeMap::new();
        let subgraph_config = &self.config.subgraphs_budget;
        let default_subgraph_max = subgraph_config.all.as_ref();
        let subgraphs_overrides = subgraph_config.subgraphs.as_ref();

        for (subgraph, estimated_cost) in evaluation.per_subgraph.as_ref() {
            let subgraph_override_max =
                subgraphs_overrides.and_then(|subgraphs| subgraphs.get(subgraph.as_str()));
            let maybe_subgraph_max = subgraph_override_max
                .or(default_subgraph_max)
                .map(|cfg| *cfg as u64);

            if let Some(subgraph_max) = maybe_subgraph_max {
                if *estimated_cost > subgraph_max {
                    debug!(
                        target: targets::DEMAND_CONTROL,
                        subgraph_name = subgraph.as_str(),
                        estimated_cost,
                        subgraph_max,
                        "subgraph call will be blocked during execution due to estimated cost exceeding limit"
                    );
                    over_limit.insert(subgraph.clone(), subgraph_max);
                }
            }
        }

        over_limit
    }

    pub(crate) fn compile_plan(
        &self,
        query_plan: &QueryPlan<Planning>,
        operation_for_plan: &OperationDefinition,
        root_type_name: &str,
        supergraph_state: &SupergraphState,
    ) -> DemandControlFormulaPlan {
        let mut actual_plans_by_fetch_hash =
            if self.config.actual_cost_mode == DemandControlActualCostMode::BySubgraph {
                Some(AHashMap::new())
            } else {
                None
            };

        let root = query_plan
            .node
            .as_ref()
            .map(|node| {
                self.compile_formula_plan_node(
                    node,
                    supergraph_state,
                    &mut actual_plans_by_fetch_hash,
                )
            })
            .unwrap_or(FormulaPlanNode::Aggregate(vec![]));

        let actual_cost_plan =
            if self.config.actual_cost_mode == DemandControlActualCostMode::BySubgraph {
                CompiledActualCostPlan::BySubgraph(
                    // Safe to unwrap because we set this up as Some if the mode is WithCompiledPlan
                    actual_plans_by_fetch_hash.unwrap(),
                )
            } else {
                CompiledActualCostPlan::ByResponseShape(compile_actual_response_shape_cost_plan(
                    operation_for_plan,
                    root_type_name,
                    supergraph_state,
                ))
            };

        DemandControlFormulaPlan {
            root,
            actual_cost_plan: Arc::new(actual_cost_plan),
        }
    }

    fn compile_formula_fetch_node(
        &self,
        service_name: &str,
        operation_kind: Option<&OperationKind>,
        operation: &PlanningFetchOperation,
        supergraph_state: &SupergraphState,
        actual_plans_by_fetch_hash: &mut Option<AHashMap<u64, CompiledSubgraphActualCostPlan>>,
    ) -> FormulaFetchNode {
        let default_list_size = self.default_list_size_for_subgraph(service_name);
        let root_type = supergraph_state.expect_root_type_name(operation_kind);
        let document = &*operation.document;
        if let Some(actual_plans_by_fetch_hash) = actual_plans_by_fetch_hash {
            actual_plans_by_fetch_hash
                .entry(operation.operation.hash)
                .or_insert_with(|| compile_actual_subgraph_cost_plan(document, supergraph_state));
        }
        FormulaFetchNode {
            service_name: service_name.to_string(),
            estimated_expr: compile_cost_expr_for_operation(
                &document.operation,
                &document.fragments,
                root_type,
                operation_kind,
                supergraph_state,
                default_list_size,
            ),
        }
    }

    fn compile_formula_plan_node(
        &self,
        node: &PlanNode<Planning>,
        supergraph_state: &SupergraphState,
        actual_plans_by_fetch_hash: &mut Option<AHashMap<u64, CompiledSubgraphActualCostPlan>>,
    ) -> FormulaPlanNode {
        let mut child = |node: &PlanNode<Planning>| {
            self.compile_formula_plan_node(node, supergraph_state, actual_plans_by_fetch_hash)
        };
        match node {
            PlanNode::Fetch(fetch) => FormulaPlanNode::Fetch(self.compile_formula_fetch_node(
                &fetch.service_name,
                fetch.operation_kind.as_ref(),
                &fetch.operation,
                supergraph_state,
                actual_plans_by_fetch_hash,
            )),
            PlanNode::BatchFetch(fetch) => FormulaPlanNode::Fetch(self.compile_formula_fetch_node(
                &fetch.service_name,
                fetch.operation_kind.as_ref(),
                &fetch.operation,
                supergraph_state,
                actual_plans_by_fetch_hash,
            )),
            PlanNode::Subscription(subscription) => {
                let fetch = &subscription.primary;
                FormulaPlanNode::Fetch(self.compile_formula_fetch_node(
                    &fetch.service_name,
                    fetch.operation_kind.as_ref(),
                    &fetch.operation,
                    supergraph_state,
                    actual_plans_by_fetch_hash,
                ))
            }
            PlanNode::Flatten(flatten) => child(&flatten.node),
            PlanNode::Sequence(sequence) => {
                FormulaPlanNode::Aggregate(sequence.nodes.iter().map(&mut child).collect())
            }
            PlanNode::Parallel(parallel) => {
                FormulaPlanNode::Aggregate(parallel.nodes.iter().map(&mut child).collect())
            }
            PlanNode::Condition(condition) => FormulaPlanNode::Condition {
                condition: condition.condition.clone(),
                if_clause: condition.if_clause.as_deref().map(&mut child).map(Box::new),
                else_clause: condition
                    .else_clause
                    .as_deref()
                    .map(&mut child)
                    .map(Box::new),
            },
            PlanNode::Defer(defer) => {
                let primary = defer.primary.node.as_deref().map(&mut child);
                let deferred = defer
                    .deferred
                    .iter()
                    .filter_map(|node| node.node.as_deref())
                    .map(&mut child)
                    .collect::<Vec<_>>();
                FormulaPlanNode::Aggregate(primary.into_iter().chain(deferred).collect())
            }
        }
    }
}
