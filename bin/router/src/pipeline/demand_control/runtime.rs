use std::collections::BTreeMap;
use std::sync::Arc;

use ahash::{HashMap as AHashMap, HashMapExt};
use hive_router_config::demand_control::{
    DemandControlActualCostMode, DemandControlConfig, DemandControlExposeHeadersConfig,
    DemandControlMode,
};
use hive_router_internal::telemetry::metrics::demand_control_metrics::DemandControlResultCode;
use hive_router_internal::telemetry::metrics::Metrics;
use hive_router_internal::telemetry::traces::spans::graphql::GraphQLSpanOperationIdentity;
use hive_router_plan_executor::execution::demand_control::{
    compile_actual_response_shape_cost_plan, compile_actual_subgraph_cost_plan,
    CompiledActualCostPlan, CompiledSubgraphActualCostPlan, DemandControlEvaluation,
    DemandControlExecutionActualCostContext, DemandControlExecutionContext,
    DemandControlExecutionOperationContext, DemandControlExecutionSubgraphsContext,
};
use hive_router_plan_executor::execution::plan::CoerceVariablesPayload;
use hive_router_plan_executor::hooks::on_supergraph_load::SupergraphData;
use hive_router_query_planner::ast::operation::{OperationDefinition, SubgraphFetchOperation};
use hive_router_query_planner::planner::plan_nodes::{PlanNode, QueryPlan};
use hive_router_query_planner::state::supergraph_state::{OperationKind, SupergraphState};
use http::{HeaderName, HeaderValue};
use moka::future::Cache;
use tracing::{debug, info, warn};

use crate::pipeline::error::PipelineError;

use super::formula::{
    compile_cost_expr_for_operation, evaluate_formula_plan, DemandControlFormulaPlan,
    FormulaFetchNode, FormulaPlanNode,
};

pub struct DemandControlRuntime {
    config: DemandControlConfig,
    expose_headers_flags: Arc<DemandControlExposeHeadersConfig>,
    metrics: Arc<Metrics>,
    formula_cache: Cache<u64, Arc<DemandControlFormulaPlan>>,
}

impl DemandControlRuntime {
    pub fn from_config(
        config: Option<&DemandControlConfig>,
        metrics: Arc<Metrics>,
    ) -> Option<Self> {
        let config = config?;
        if !config.enabled {
            debug!("demand control is disabled");
            return None;
        }

        info!(
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
                    "demand control is in enforce mode with a max cost of 0; all operations with non-zero cost will be rejected"
                );
            }

            if config.default_list_size.all.is_none()
                && config.default_list_size.subgraphs.is_none()
            {
                warn!(
                    "demand control is in enforce mode without a default list_size; list fields without an @listSize directive are estimated as 0 and may be under-counted"
                );
            }
        }

        Some(Self {
            expose_headers_flags: Arc::new(config.operation_cost.expose_headers.clone()),
            config: config.clone(),
            metrics,
            formula_cache: Cache::new(1000),
        })
    }

    pub fn formula_cache(&self) -> &Cache<u64, Arc<DemandControlFormulaPlan>> {
        &self.formula_cache
    }

    pub fn invalidate_formula_cache(&self) {
        self.formula_cache.invalidate_all();
    }
}

impl DemandControlRuntime {
    #[allow(clippy::too_many_arguments)]
    pub async fn evaluate<'exec>(
        &self,
        supergraph: &'exec SupergraphData,
        variable_payload: &'exec CoerceVariablesPayload,
        query_plan: &'exec QueryPlan,
        operation_for_plan: &'exec OperationDefinition,
        root_type_name: &'exec str,
        normalized_operation_hash: u64,
        operation_identity: GraphQLSpanOperationIdentity<'exec>,
    ) -> Result<DemandControlExecutionContext, PipelineError> {
        let operation_name = operation_identity.name;
        let compiled_plan = self
            .formula_cache
            .entry(normalized_operation_hash)
            .or_insert_with(async {
                Arc::new(self.compile_demand_control_plan(
                    query_plan,
                    operation_for_plan,
                    root_type_name,
                    &supergraph.planner.supergraph,
                ))
            })
            .await
            .into_value();

        let evaluation = evaluate_formula_plan(
            compiled_plan.as_ref(),
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

                    return Err(PipelineError::CostEstimatedTooExpensive {
                        response_headers: err_extra_headers,
                    });
                }
                DemandControlMode::Measure => {
                    info!(
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
                    debug!(subgraph_name = subgraph.as_str(), estimated_cost, subgraph_max, "subgraph call will be blocked dueing execution due to estimated cost exceeding limit");
                    over_limit.insert(subgraph.clone(), subgraph_max);
                }
            }
        }

        over_limit
    }

    fn compile_demand_control_plan(
        &self,
        query_plan: &QueryPlan,
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
        operation: &SubgraphFetchOperation,
        supergraph_state: &SupergraphState,
        actual_plans_by_fetch_hash: &mut Option<AHashMap<u64, CompiledSubgraphActualCostPlan>>,
    ) -> FormulaFetchNode {
        let default_list_size = self.default_list_size_for_subgraph(service_name);
        let root_type = supergraph_state.root_type_name(operation_kind);
        if let Some(actual_plans_by_fetch_hash) = actual_plans_by_fetch_hash {
            actual_plans_by_fetch_hash
                .entry(operation.hash)
                .or_insert_with(|| compile_actual_subgraph_cost_plan(operation, supergraph_state));
        }
        FormulaFetchNode {
            service_name: service_name.to_string(),
            estimated_expr: compile_cost_expr_for_operation(
                &operation.document.operation,
                &operation.document.fragments,
                root_type,
                operation_kind,
                supergraph_state,
                default_list_size,
            ),
        }
    }

    fn compile_formula_plan_node(
        &self,
        node: &PlanNode,
        supergraph_state: &SupergraphState,
        actual_plans_by_fetch_hash: &mut Option<AHashMap<u64, CompiledSubgraphActualCostPlan>>,
    ) -> FormulaPlanNode {
        match node {
            PlanNode::Fetch(fetch_node) => FormulaPlanNode::Fetch(self.compile_formula_fetch_node(
                &fetch_node.service_name,
                fetch_node.operation_kind.as_ref(),
                &fetch_node.operation,
                supergraph_state,
                actual_plans_by_fetch_hash,
            )),
            PlanNode::BatchFetch(batch_fetch_node) => {
                FormulaPlanNode::Fetch(self.compile_formula_fetch_node(
                    &batch_fetch_node.service_name,
                    batch_fetch_node.operation_kind.as_ref(),
                    &batch_fetch_node.operation,
                    supergraph_state,
                    actual_plans_by_fetch_hash,
                ))
            }
            PlanNode::Flatten(flatten) => self.compile_formula_plan_node(
                &flatten.node,
                supergraph_state,
                actual_plans_by_fetch_hash,
            ),
            PlanNode::Sequence(sequence) => FormulaPlanNode::Aggregate(
                sequence
                    .nodes
                    .iter()
                    .map(|child| {
                        self.compile_formula_plan_node(
                            child,
                            supergraph_state,
                            actual_plans_by_fetch_hash,
                        )
                    })
                    .collect(),
            ),
            PlanNode::Parallel(parallel) => FormulaPlanNode::Aggregate(
                parallel
                    .nodes
                    .iter()
                    .map(|child| {
                        self.compile_formula_plan_node(
                            child,
                            supergraph_state,
                            actual_plans_by_fetch_hash,
                        )
                    })
                    .collect(),
            ),
            PlanNode::Condition(condition) => FormulaPlanNode::Condition {
                condition: condition.condition.clone(),
                if_clause: condition.if_clause.as_ref().map(|node| {
                    Box::new(self.compile_formula_plan_node(
                        node,
                        supergraph_state,
                        actual_plans_by_fetch_hash,
                    ))
                }),
                else_clause: condition.else_clause.as_ref().map(|node| {
                    Box::new(self.compile_formula_plan_node(
                        node,
                        supergraph_state,
                        actual_plans_by_fetch_hash,
                    ))
                }),
            },
            PlanNode::Subscription(subscription) => {
                FormulaPlanNode::Fetch(self.compile_formula_fetch_node(
                    &subscription.primary.service_name,
                    subscription.primary.operation_kind.as_ref(),
                    &subscription.primary.operation,
                    supergraph_state,
                    actual_plans_by_fetch_hash,
                ))
            }
            PlanNode::Defer(defer) => {
                let primary = defer.primary.node.as_ref().map(|primary| {
                    self.compile_formula_plan_node(
                        primary,
                        supergraph_state,
                        actual_plans_by_fetch_hash,
                    )
                });
                let deferred: Vec<FormulaPlanNode> = defer
                    .deferred
                    .iter()
                    .filter_map(|node| node.node.as_ref())
                    .map(|node| {
                        self.compile_formula_plan_node(
                            node,
                            supergraph_state,
                            actual_plans_by_fetch_hash,
                        )
                    })
                    .collect();
                let aggregate = primary.into_iter().chain(deferred).collect();
                FormulaPlanNode::Aggregate(aggregate)
            }
        }
    }
}
