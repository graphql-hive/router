use std::collections::BTreeMap;
use std::sync::Arc;

use ahash::{HashMap as AHashMap, HashMapExt};
use hive_router_config::demand_control::{
    DemandControlActualCostMode, DemandControlConfig, DemandControlMode,
};
use hive_router_internal::telemetry::metrics::demand_control_metrics::DemandControlResultCode;
use hive_router_internal::telemetry::metrics::Metrics;
use hive_router_internal::telemetry::traces::spans::graphql::GraphQLSpanOperationIdentity;
use hive_router_plan_executor::execution::demand_control::{
    compile_actual_response_shape_cost_plan, compile_actual_subgraph_cost_plan,
    CompiledActualCostPlan, CompiledSubgraphActualCostPlan, DemandControlEvaluation,
    DemandControlExecutionContext,
};
use hive_router_plan_executor::execution::plan::CoerceVariablesPayload;
use hive_router_plan_executor::hooks::on_supergraph_load::SupergraphData;
use hive_router_query_planner::ast::operation::{OperationDefinition, SubgraphFetchOperation};
use hive_router_query_planner::planner::plan_nodes::{PlanNode, QueryPlan};
use hive_router_query_planner::state::supergraph_state::{OperationKind, SupergraphState};
use moka::future::Cache;
use tracing::{debug, info, warn};

use crate::cache_state::{CacheHitMiss, EntryValueHitMissExt};
use crate::pipeline::error::PipelineError;

use super::formula::{
    collect_estimated_formulas, compile_cost_expr_for_operation, evaluate_formula_plan,
    DemandControlFormulaPlan, FormulaFetchNode, FormulaPlanNode,
};

pub struct DemandControlRuntime {
    config: DemandControlConfig,
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

        let static_estimated = config.strategy.static_estimated();
        info!(
            mode = ?config.mode,
            max_cost = static_estimated.max,
            default_list_size = ?static_estimated.list_size,
            actual_cost_mode = ?static_estimated.actual_cost_mode,
            "demand control enabled"
        );

        if config.mode == DemandControlMode::Enforce {
            if static_estimated.max == 0 {
                warn!(
                    "demand control is in enforce mode with a max cost of 0; all operations with non-zero cost will be rejected"
                );
            }
            if static_estimated.list_size.is_none() {
                warn!(
                    "demand control is in enforce mode without a default list_size; list fields without an @listSize directive are estimated as 0 and may be under-counted"
                );
            }
        }

        Some(Self {
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
        let mut formula_cache_hit = true;

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
            .into_value_with_hit_miss(|hit_miss| {
                formula_cache_hit = matches!(hit_miss, CacheHitMiss::Hit);
            });

        let estimation = evaluate_formula_plan(
            compiled_plan.as_ref(),
            &supergraph.planner.supergraph,
            variable_payload,
        )?;

        let max_cost = self.config.strategy.static_estimated().max;
        let estimated_exceeds_max = estimation.estimated_cost > max_cost;
        let subgraphs_exceed_limits = self.subgraphs_over_limit(&estimation);

        if estimated_exceeds_max {
            match self.config.mode {
                DemandControlMode::Enforce => {
                    warn!(
                        operation_name = ?operation_name,
                        estimated_cost = estimation.estimated_cost,
                        max_cost,
                        "rejecting operation: estimated cost exceeds configured max cost"
                    );

                    let mut formulas = AHashMap::new();
                    collect_estimated_formulas(&compiled_plan.root, &mut formulas);

                    for (subgraph, formula) in &formulas {
                        debug!(
                            operation_name = ?operation_name,
                            subgraph = subgraph.as_str(),
                            %formula,
                            "demand control cost formula for rejected operation"
                        );
                    }

                    self.metrics.demand_control.record_estimated_cost(
                        estimation.estimated_cost,
                        &DemandControlResultCode::CostEstimatedTooExpensive,
                        operation_name,
                    );
                    return Err(PipelineError::CostEstimatedTooExpensive {
                        estimated_cost: estimation.estimated_cost,
                        max_cost,
                    });
                }
                DemandControlMode::Measure => {
                    debug!(
                        operation_name = ?operation_name,
                        estimated_cost = estimation.estimated_cost,
                        max_cost,
                        "measure mode: operation would be rejected in enforce mode"
                    );
                }
            }
        }

        let estimated_result = if estimated_exceeds_max {
            DemandControlResultCode::CostEstimatedTooExpensive
        } else {
            DemandControlResultCode::CostOk
        };

        self.metrics.demand_control.record_estimated_cost(
            estimation.estimated_cost,
            &estimated_result,
            operation_name,
        );

        Ok(DemandControlExecutionContext {
            mode: self.config.mode,
            max_cost,
            evaluation: estimation,
            subgraphs_over_limit: subgraphs_exceed_limits,
            actual_cost_mode: self.config.strategy.static_estimated().actual_cost_mode,
            result_code: estimated_result,
            metrics_recorder: self.metrics.demand_control.recorder(),
            include_extension_metadata: self.config.include_extension_metadata.unwrap_or(false),
            formula_cache_hit,
            estimated_formula_by_subgraph: compiled_plan.formula_by_subgraph.clone(),
            actual_cost_plan: compiled_plan.actual_cost_plan.clone(),
        })
    }
}

impl DemandControlRuntime {
    fn default_list_size_for_subgraph(&self, subgraph_name: &str) -> usize {
        let se = self.config.strategy.static_estimated();
        se.subgraph
            .subgraphs
            .as_ref()
            .and_then(|subgraphs| subgraphs.get(subgraph_name))
            .and_then(|cfg| cfg.list_size)
            .or_else(|| se.subgraph.all.as_ref().and_then(|cfg| cfg.list_size))
            .or(se.list_size)
            .unwrap_or(0)
    }

    fn subgraphs_over_limit(
        &self,
        evaluation: &DemandControlEvaluation,
    ) -> std::collections::BTreeMap<String, u64> {
        let mut over_limit = std::collections::BTreeMap::new();
        let subgraph_config = &self.config.strategy.static_estimated().subgraph;

        let inherited_max = subgraph_config.all.as_ref().and_then(|cfg| cfg.max);

        for (subgraph, estimated_cost) in evaluation.per_subgraph.as_ref() {
            let specific_max = subgraph_config
                .subgraphs
                .as_ref()
                .and_then(|subgraphs| subgraphs.get(subgraph.as_str()))
                .and_then(|cfg| cfg.max);
            let max = specific_max.or(inherited_max);

            if let Some(limit) = max {
                if *estimated_cost > limit {
                    over_limit.insert(subgraph.clone(), limit);
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
        let include_extension_metadata = self.config.include_extension_metadata.unwrap_or(false);

        let mut actual_plans_by_fetch_hash =
            if self.config.strategy.static_estimated().actual_cost_mode
                == DemandControlActualCostMode::BySubgraph
            {
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

        let formula_by_subgraph = if include_extension_metadata {
            let mut expr_by_subgraph = AHashMap::new();
            collect_estimated_formulas(&root, &mut expr_by_subgraph);
            expr_by_subgraph
                .into_iter()
                .map(|(service, expr)| (service, expr.to_string()))
                .collect::<BTreeMap<_, _>>()
        } else {
            BTreeMap::new()
        };

        let actual_cost_plan = if self.config.strategy.static_estimated().actual_cost_mode
            == DemandControlActualCostMode::BySubgraph
        {
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
            formula_by_subgraph: Arc::new(formula_by_subgraph),
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
