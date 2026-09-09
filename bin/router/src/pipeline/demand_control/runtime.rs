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
use crate::query_planner::ast::document::{Document, DocumentParseError};
use crate::query_planner::ast::operation::OperationDefinition;
use crate::query_planner::planner::plan_nodes::{PlanNode, PlanState, Planning, QueryPlan};
use crate::query_planner::state::supergraph_state::{OperationKind, SupergraphState};
use crate::telemetry::logging::targets;
use crate::telemetry::metrics::demand_control_metrics::DemandControlResultCode;
use crate::telemetry::metrics::Metrics;
use crate::telemetry::traces::spans::graphql::GraphQLSpanOperationIdentity;
use ahash::{HashMap as AHashMap, HashMapExt};
use http::{HeaderName, HeaderValue};
use std::{borrow::Cow, convert::Infallible};
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

    /// Compiled once per plan-cache entry, while the plan is being built. The result is stored
    /// with the plan, so it is keyed exactly like the plan it describes.
    ///
    /// A planning plan still carries its parsed documents, so this cannot fail.
    pub(crate) fn compile_plan(
        &self,
        query_plan: &QueryPlan<Planning>,
        operation_for_plan: &OperationDefinition,
        root_type_name: &str,
        supergraph_state: &SupergraphState,
    ) -> DemandControlFormulaPlan {
        match self.compile_plan_with_documents(
            query_plan,
            operation_for_plan,
            root_type_name,
            supergraph_state,
            |operation| Ok::<_, Infallible>(Cow::Borrowed(&*operation.document)),
        ) {
            Ok(plan) => plan,
            Err(never) => match never {},
        }
    }

    /// A plan a plugin replaced has only operation text left, so each fetch is parsed back while
    /// its costs are compiled. Unparseable text is reported, never costed as zero.
    pub(crate) fn compile_executable_plan(
        &self,
        query_plan: &QueryPlan,
        operation_for_plan: &OperationDefinition,
        root_type_name: &str,
        supergraph_state: &SupergraphState,
    ) -> Result<DemandControlFormulaPlan, DocumentParseError> {
        self.compile_plan_with_documents(
            query_plan,
            operation_for_plan,
            root_type_name,
            supergraph_state,
            |operation| Document::parse_executable(&operation.document_str).map(Cow::Owned),
        )
    }

    fn compile_plan_with_documents<S: PlanState, E>(
        &self,
        query_plan: &QueryPlan<S>,
        operation_for_plan: &OperationDefinition,
        root_type_name: &str,
        supergraph_state: &SupergraphState,
        document: fn(&S::Operation) -> Result<Cow<'_, Document>, E>,
    ) -> Result<DemandControlFormulaPlan, E> {
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
                    document,
                )
            })
            .transpose()?
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

        Ok(DemandControlFormulaPlan {
            root,
            actual_cost_plan: Arc::new(actual_cost_plan),
        })
    }

    fn compile_formula_fetch_node<S: PlanState, E>(
        &self,
        service_name: &str,
        operation_kind: Option<&OperationKind>,
        operation: &S::Operation,
        supergraph_state: &SupergraphState,
        actual_plans_by_fetch_hash: &mut Option<AHashMap<u64, CompiledSubgraphActualCostPlan>>,
        document: fn(&S::Operation) -> Result<Cow<'_, Document>, E>,
    ) -> Result<FormulaFetchNode, E> {
        let default_list_size = self.default_list_size_for_subgraph(service_name);
        let root_type = supergraph_state.expect_root_type_name(operation_kind);
        let document = document(operation)?;
        if let Some(actual_plans_by_fetch_hash) = actual_plans_by_fetch_hash {
            actual_plans_by_fetch_hash
                .entry(operation.as_ref().hash)
                .or_insert_with(|| compile_actual_subgraph_cost_plan(&document, supergraph_state));
        }
        Ok(FormulaFetchNode {
            service_name: service_name.to_string(),
            estimated_expr: compile_cost_expr_for_operation(
                &document.operation,
                &document.fragments,
                root_type,
                operation_kind,
                supergraph_state,
                default_list_size,
            ),
        })
    }

    fn compile_formula_plan_node<S: PlanState, E>(
        &self,
        node: &PlanNode<S>,
        supergraph_state: &SupergraphState,
        actual_plans_by_fetch_hash: &mut Option<AHashMap<u64, CompiledSubgraphActualCostPlan>>,
        document: fn(&S::Operation) -> Result<Cow<'_, Document>, E>,
    ) -> Result<FormulaPlanNode, E> {
        // Recursive calls share the same document source: borrowed while planning, parsed for replacements.
        let mut child = |node: &PlanNode<S>| {
            self.compile_formula_plan_node(
                node,
                supergraph_state,
                actual_plans_by_fetch_hash,
                document,
            )
        };
        Ok(match node {
            PlanNode::Fetch(fetch) => {
                FormulaPlanNode::Fetch(self.compile_formula_fetch_node::<S, E>(
                    &fetch.service_name,
                    fetch.operation_kind.as_ref(),
                    &fetch.operation,
                    supergraph_state,
                    actual_plans_by_fetch_hash,
                    document,
                )?)
            }
            PlanNode::BatchFetch(fetch) => {
                FormulaPlanNode::Fetch(self.compile_formula_fetch_node::<S, E>(
                    &fetch.service_name,
                    fetch.operation_kind.as_ref(),
                    &fetch.operation,
                    supergraph_state,
                    actual_plans_by_fetch_hash,
                    document,
                )?)
            }
            PlanNode::Subscription(subscription) => {
                let fetch = &subscription.primary;
                FormulaPlanNode::Fetch(self.compile_formula_fetch_node::<S, E>(
                    &fetch.service_name,
                    fetch.operation_kind.as_ref(),
                    &fetch.operation,
                    supergraph_state,
                    actual_plans_by_fetch_hash,
                    document,
                )?)
            }
            PlanNode::Flatten(flatten) => child(&flatten.node)?,
            PlanNode::Sequence(sequence) => FormulaPlanNode::Aggregate(
                sequence
                    .nodes
                    .iter()
                    .map(&mut child)
                    .collect::<Result<_, _>>()?,
            ),
            PlanNode::Parallel(parallel) => FormulaPlanNode::Aggregate(
                parallel
                    .nodes
                    .iter()
                    .map(&mut child)
                    .collect::<Result<_, _>>()?,
            ),
            PlanNode::Condition(condition) => FormulaPlanNode::Condition {
                condition: condition.condition.clone(),
                if_clause: condition
                    .if_clause
                    .as_deref()
                    .map(&mut child)
                    .transpose()?
                    .map(Box::new),
                else_clause: condition
                    .else_clause
                    .as_deref()
                    .map(&mut child)
                    .transpose()?
                    .map(Box::new),
            },
            PlanNode::Defer(defer) => {
                let primary = defer.primary.node.as_deref().map(&mut child).transpose()?;
                let deferred = defer
                    .deferred
                    .iter()
                    .filter_map(|node| node.node.as_deref())
                    .map(&mut child)
                    .collect::<Result<Vec<_>, _>>()?;
                FormulaPlanNode::Aggregate(primary.into_iter().chain(deferred).collect())
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query_planner::ast::normalization::normalize_operation;
    use crate::query_planner::graph::PlannerOverrideContext;
    use crate::query_planner::planner::{Planner, QueryPlannerOptions};
    use crate::query_planner::utils::cancellation::CancellationToken;
    use crate::query_planner::utils::parsing::{parse_operation, parse_schema};

    fn runtime() -> DemandControlRuntime {
        runtime_with_mode("by_subgraph")
    }

    fn runtime_with_mode(actual_cost_mode: &str) -> DemandControlRuntime {
        let config: DemandControlConfig = serde_json::from_value(serde_json::json!({
            "enabled": true,
            "operation_cost": { "max": 1_000_000u64, "mode": "measure" },
            "subgraphs_budget": { "mode": "measure", "all": null },
            "actual_cost_mode": actual_cost_mode,
        }))
        .expect("valid demand control config");

        DemandControlRuntime::from_config(Some(&config), Arc::new(Metrics::new(None)))
            .expect("demand control is enabled")
    }

    fn variables(pairs: serde_json::Value) -> CoerceVariablesPayload {
        let object = pairs.as_object().expect("variables are an object").clone();
        CoerceVariablesPayload {
            variables_map: Some(
                object
                    .into_iter()
                    .map(|(name, value)| {
                        (
                            name,
                            sonic_rs::from_str(&value.to_string()).expect("variable value"),
                        )
                    })
                    .collect(),
            ),
        }
    }

    fn attributed_subgraphs(node: &FormulaPlanNode, out: &mut Vec<String>) {
        match node {
            FormulaPlanNode::Fetch(fetch) => out.push(fetch.service_name.clone()),
            FormulaPlanNode::Aggregate(children) => {
                children.iter().for_each(|c| attributed_subgraphs(c, out))
            }
            FormulaPlanNode::Condition {
                if_clause,
                else_clause,
                ..
            } => {
                if let Some(if_clause) = if_clause {
                    attributed_subgraphs(if_clause, out);
                }
                if let Some(else_clause) = else_clause {
                    attributed_subgraphs(else_clause, out);
                }
            }
        }
    }

    /// The formula used to be cached on the normalized operation hash alone, while the plan it
    /// describes is cached on that hash plus the override context. Two override buckets therefore
    /// shared whichever formula compiled first, including its per-subgraph attribution.
    #[test]
    fn override_context_changes_the_compiled_cost_plan() {
        let sdl = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("fixture/tests/simple-progressive-overrides.supergraph.graphql"),
        )
        .expect("fixture is readable");
        let schema = parse_schema(&sdl);
        let planner = Planner::new_from_supergraph(&schema, QueryPlannerOptions::default())
            .expect("planner builds");

        let document = parse_operation("{ aFeed { createdAt } bFeed { createdAt } }");
        let normalized = normalize_operation(&planner.supergraph, &document, None)
            .expect("operation normalizes");
        let root_type_name = planner
            .supergraph
            .expect_root_type_name(Some(&OperationKind::Query));

        let runtime = runtime();
        let compile = |percentage: f64| {
            let plan = planner
                .plan_from_normalized_operation(
                    &normalized.operation,
                    PlannerOverrideContext::from_percentage(percentage),
                    &CancellationToken::new(),
                )
                .expect("plans");
            let compiled = runtime.compile_plan(
                &plan,
                &normalized.operation,
                root_type_name,
                &planner.supergraph,
            );
            let mut subgraphs = Vec::new();
            attributed_subgraphs(&compiled.root, &mut subgraphs);
            subgraphs
        };

        let below = compile(50.0);
        let above = compile(90.0);

        assert!(
            !below.is_empty() && !above.is_empty(),
            "both override contexts should produce cost formulas"
        );
        assert_ne!(
            below, above,
            "cost attribution must follow the plan the override context produced; sharing one \
             formula across override buckets is the bug this guards"
        );
    }

    #[test]
    fn replacement_costing_propagates_invalid_operation_text() {
        use crate::query_planner::ast::operation::PlanningFetchOperation;
        use crate::query_planner::planner::plan_nodes::*;
        let sdl = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../e2e/supergraph_demand_control.graphql"),
        )
        .unwrap();
        let planner =
            Planner::new_from_supergraph(&parse_schema(&sdl), QueryPlannerOptions::default())
                .unwrap();
        let document = parse_operation("{ bestsellers { title } }");
        let normalized = normalize_operation(&planner.supergraph, &document, None).unwrap();
        let plan = planner
            .plan_from_normalized_operation(
                &normalized.operation,
                PlannerOverrideContext::default(),
                &CancellationToken::new(),
            )
            .unwrap()
            .into_executable();
        let runtime = runtime_with_mode("by_subgraph");
        let root = planner
            .supergraph
            .expect_root_type_name(Some(&OperationKind::Query));
        let compile = |plan: &QueryPlan| {
            runtime.compile_executable_plan(plan, &normalized.operation, root, &planner.supergraph)
        };
        assert!(compile(&plan).is_ok());
        // Put invalid text after a valid subtree: no successful partial formula may escape.
        let operation = PlanningFetchOperation::from_anonymous_operation(
            Document::parse_executable("{ __typename }").unwrap(),
        );
        let mut operation = operation.operation;
        operation.document_str = "{ invalid".into();
        let invalid = PlanNode::Fetch(Box::new(FetchNode {
            id: -1,
            service_name: "books".into(),
            variable_usages: None,
            operation_kind: Some(OperationKind::Query),
            operation,
            custom_scalar_paths: None,
            requires: None,
            input_rewrites: None,
            output_rewrites: None,
        }));
        let replacement = QueryPlan {
            kind: plan.kind,
            node: Some(PlanNode::Sequence(SequenceNode {
                nodes: vec![plan.node.unwrap(), invalid],
            })),
        };
        assert!(compile(&replacement).is_err());
    }

    #[test]
    fn executable_plan_costs_match_planning_costs() {
        let sdl = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../e2e/supergraph_demand_control.graphql"),
        )
        .expect("the demand-control supergraph fixture is readable");
        let schema = parse_schema(&sdl);
        let planner = Planner::new_from_supergraph(&schema, QueryPlannerOptions::default())
            .expect("planner builds");

        let document = parse_operation(
            r#"query($limit: Int!, $withBio: Boolean!) {
                newestAdditions(limit: $limit) { title }
                bookWithArgCost(limit: 3) { title }
                bookWithFieldCost { title }
                bestsellers { title author { bio @include(if: $withBio) } }
            }"#,
        );
        let normalized = normalize_operation(&planner.supergraph, &document, None)
            .expect("operation normalizes");
        let root_type_name = planner
            .supergraph
            .expect_root_type_name(Some(&OperationKind::Query));

        let variable_sets = [
            variables(serde_json::json!({ "limit": 7, "withBio": true })),
            variables(serde_json::json!({ "limit": 40, "withBio": false })),
        ];

        for mode in ["by_subgraph", "by_response_shape"] {
            let runtime = runtime_with_mode(mode);
            let plan = planner
                .plan_from_normalized_operation(
                    &normalized.operation,
                    PlannerOverrideContext::default(),
                    &CancellationToken::new(),
                )
                .expect("plans");

            let compile = |plan: &_| {
                runtime.compile_plan(
                    plan,
                    &normalized.operation,
                    root_type_name,
                    &planner.supergraph,
                )
            };
            let evaluate = |compiled: &DemandControlFormulaPlan, vars| {
                evaluate_formula_plan(compiled, &planner.supergraph, vars)
                    .expect("formula evaluates")
            };

            let as_planned = compile(&plan);
            let costs_as_planned: Vec<_> = variable_sets
                .iter()
                .map(|vars| {
                    let evaluated = evaluate(&as_planned, vars);
                    (evaluated.estimated_cost, (*evaluated.per_subgraph).clone())
                })
                .collect();

            let plan = plan.into_executable();
            let restored = runtime
                .compile_executable_plan(
                    &plan,
                    &normalized.operation,
                    root_type_name,
                    &planner.supergraph,
                )
                .expect("executable operation text parses");

            for (vars, (expected_total, expected_per_subgraph)) in
                variable_sets.iter().zip(&costs_as_planned)
            {
                let evaluated = evaluate(&restored, vars);
                assert_eq!(
                    evaluated.estimated_cost, *expected_total,
                    "estimated total changed after reconstruction in {mode} mode"
                );
                assert_eq!(
                    &*evaluated.per_subgraph, expected_per_subgraph,
                    "per-subgraph costs changed after reconstruction in {mode} mode"
                );
            }

            assert_eq!(
                format!("{:?}", as_planned.actual_cost_plan),
                format!("{:?}", restored.actual_cost_plan),
                "the compiled actual-cost plan changed after reconstruction in {mode} mode"
            );

            let (total, per_subgraph) = &costs_as_planned[0];
            assert!(
                *total > 0 && !per_subgraph.is_empty(),
                "the fixture must produce a non-zero cost to compare"
            );
            assert_ne!(
                costs_as_planned[0].0, costs_as_planned[1].0,
                "variables must change the estimate, or this test proves nothing about them"
            );
        }
    }
}
