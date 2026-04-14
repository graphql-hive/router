use ahash::{HashMap as AHashMap, HashMapExt, HashSet as AHashSet, HashSetExt};

use hive_router_internal::telemetry::metrics::demand_control_metrics::DemandControlResultCode;
use hive_router_internal::telemetry::traces::spans::graphql::{
    GraphQLDemandControlSpan, GraphQLSpanOperationIdentity,
};
use hive_router_plan_executor::execution::plan::CoerceVariablesPayload;
use hive_router_plan_executor::{
    execution::demand_control::{
        ActualCostPlanField, ActualCostPlanNode, DemandControlEvaluation,
        DemandControlExecutionContext,
    },
    hooks::on_supergraph_load::SupergraphData,
};
use hive_router_query_planner::{
    ast::{
        fragment::FragmentDefinition,
        operation::OperationDefinition,
        selection_item::SelectionItem,
        selection_set::{FieldSelection, SelectionSet},
        value::Value as AstValue,
    },
    planner::plan_nodes::{PlanNode, QueryPlan},
    state::supergraph_state::{OperationKind, SupergraphDefinition, SupergraphState, TypeNode},
};
use sonic_rs::{JsonContainerTrait, JsonValueTrait, Value};

use std::{fmt, sync::Arc, time::Instant};
use tracing::Instrument;

use crate::{
    cache_state::{CacheHitMiss, EntryValueHitMissExt},
    pipeline::error::PipelineError,
    schema_state::SchemaState,
    shared_state::RouterSharedState,
};

// ── CostExpr compilation as a mathematical formula for estimated cost ───────────────────────

/// Size overrides threaded from a parent @listSize(sizedFields:[…]) down to children.
/// Each entry is (remaining path from the current selection, size expression).
type SizeOverrides = Vec<(Vec<String>, CostExpr)>;

/// A mathematical cost expression compiled once per query shape.
/// Evaluated with only variable lookups, so no schema traversal at request time.
#[derive(Clone, Debug)]
enum CostExpr {
    /// Compile-time constant.
    Const(u64),
    /// Sum of child expressions.
    Add(Vec<CostExpr>),
    /// Product: typically `list_size × per_item_cost`.
    Mul(Box<CostExpr>, Box<CostExpr>),
    /// Conditional on a boolean request variable (@skip / @include).
    Cond {
        variable: String,
        if_true: Box<CostExpr>,
        if_false: Box<CostExpr>,
    },
    /// Integer resolved from request variables (list size via slicing arguments).
    ListSize {
        args: Vec<(AstValue, Vec<String>)>, // (argument value, path within value)
        require_one: bool,
        default: usize,
    },
    /// Cost of an input-object argument evaluated from the variable value at request time.
    InputArgCost {
        value: AstValue,
        value_type: TypeNode,
    },
}

impl CostExpr {
    /// Build an Add, collapsing Const(0) entries and single-element lists.
    fn add_nonzero(exprs: Vec<Self>) -> Self {
        let nonzero: Vec<Self> = exprs
            .into_iter()
            .filter(|e| !matches!(e, Self::Const(0)))
            .collect();
        match nonzero.len() {
            0 => Self::Const(0),
            1 => nonzero.into_iter().next().unwrap(),
            _ => Self::Add(nonzero),
        }
    }

    /// Build a Mul with constant-folding.
    fn mul(lhs: Self, rhs: Self) -> Self {
        match (&lhs, &rhs) {
            (Self::Const(0), _) | (_, Self::Const(0)) => Self::Const(0),
            (Self::Const(1), _) => rhs,
            (_, Self::Const(1)) => lhs,
            _ => Self::Mul(Box::new(lhs), Box::new(rhs)),
        }
    }
}

impl fmt::Display for CostExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Const(n) => write!(f, "{n}"),
            Self::Add(exprs) => {
                write!(f, "(")?;
                for (i, e) in exprs.iter().enumerate() {
                    if i > 0 {
                        write!(f, " + ")?;
                    }
                    write!(f, "{e}")?;
                }
                write!(f, ")")
            }
            Self::Mul(lhs, rhs) => write!(f, "({lhs} * {rhs})"),
            Self::Cond {
                variable,
                if_true,
                if_false,
            } => {
                write!(f, "(if ${variable} then {if_true} else {if_false})")
            }
            Self::ListSize {
                args,
                require_one,
                default,
            } => {
                let parts: Vec<String> = args
                    .iter()
                    .map(|(v, path)| match v {
                        AstValue::Variable(name) if path.is_empty() => format!("${name}"),
                        AstValue::Variable(name) => format!("${name}.{}", path.join(".")),
                        AstValue::Int(n) => n.to_string(),
                        _ => "?".to_string(),
                    })
                    .collect();
                if parts.len() == 1 {
                    write!(f, "{}", parts[0])
                } else if *require_one {
                    write!(f, "exactlyOne([{}], default={default})", parts.join(", "))
                } else {
                    write!(f, "max([{}], default={default})", parts.join(", "))
                }
            }
            Self::InputArgCost { value, .. } => match value {
                AstValue::Variable(v) => write!(f, "inputCost(${v})"),
                AstValue::Object(_) => write!(f, "inputCost({{...}})"),
                _ => write!(f, "inputCost(...)"),
            },
        }
    }
}

// ── Compiled plan types ──────────────────────────────────────────────────────

pub struct DemandControlFormulaPlan {
    root: FormulaPlanNode,
    formula_by_subgraph: Arc<AHashMap<String, String>>,
}

enum FormulaPlanNode {
    Empty,
    Fetch(FormulaFetchNode),
    Sequence(Vec<FormulaPlanNode>),
    Parallel(Vec<FormulaPlanNode>),
    Condition {
        condition: String,
        if_clause: Option<Box<FormulaPlanNode>>,
        else_clause: Option<Box<FormulaPlanNode>>,
    },
    Subscription(Box<FormulaPlanNode>),
    Defer {
        primary: Option<Box<FormulaPlanNode>>,
        deferred: Vec<FormulaPlanNode>,
    },
}

struct FormulaFetchNode {
    service_name: String,
    /// Mathematical formula: pure arithmetic on variable lookups.
    estimated_expr: CostExpr,
    /// Pre-compiled actual-cost nodes for schema-lookup-free response traversal.
    actual_nodes: Arc<Vec<ActualCostPlanNode>>,
}

pub async fn evaluate_demand_control<'exec>(
    app_state: &'exec RouterSharedState,
    schema_state: &'exec SchemaState,
    supergraph: &'exec SupergraphData,
    variable_payload: &'exec CoerceVariablesPayload,
    query_plan: &'exec QueryPlan,
    normalized_operation_hash: u64,
    operation_identity: GraphQLSpanOperationIdentity<'exec>,
) -> Result<Option<DemandControlExecutionContext>, PipelineError> {
    let Some(config) = &app_state.router_config.demand_control else {
        return Ok(None);
    };

    if !config.enabled {
        return Ok(None);
    }

    let demand_control_span = GraphQLDemandControlSpan::new();
    let operation_name = operation_identity.name;
    demand_control_span.record_operation_identity(operation_identity);
    async {
        let metrics = &app_state.telemetry_context.metrics;
        let formula_cache_capture = metrics.cache.demand_control_formula.capture_request();
        let mut formula_cache_hit = true;

        let compiled_plan = schema_state
            .demand_control_formula_cache
            .entry(normalized_operation_hash)
            .or_insert_with(async {
                let compile_started_at = Instant::now();
                let compiled = Arc::new(compile_demand_control_plan(
                    query_plan,
                    &supergraph.planner.supergraph,
                    config,
                ));
                demand_control_span
                    .record_compile_ms(compile_started_at.elapsed().as_secs_f64() * 1000.0);
                compiled
            })
            .await
            .into_value_with_hit_miss(|hit_miss| match hit_miss {
                CacheHitMiss::Hit => {
                    formula_cache_hit = true;
                    demand_control_span.record_cache_hit(true);
                    formula_cache_capture.finish_hit();
                }
                CacheHitMiss::Miss | CacheHitMiss::Error => {
                    formula_cache_hit = false;
                    demand_control_span.record_cache_hit(false);
                    formula_cache_capture.finish_miss();
                }
            });

        let eval_started_at = Instant::now();
        let estimation = evaluate_compiled_plan(
            compiled_plan.as_ref(),
            &supergraph.planner.supergraph,
            variable_payload,
        );
        demand_control_span.record_eval_ms(eval_started_at.elapsed().as_secs_f64() * 1000.0);

        if let Some(max_cost) = config.max_cost {
            if estimation.estimated_cost > max_cost {
                metrics.demand_control.record_estimated_cost(
                    estimation.estimated_cost,
                    &DemandControlResultCode::CostEstimatedTooExpensive,
                    operation_name,
                );
                return Err(PipelineError::CostEstimatedTooExpensive {
                    estimated_cost: estimation.estimated_cost,
                    max_cost,
                });
            }
        }

        let blocked_subgraphs = blocked_subgraphs(config, &estimation);

        let estimated_result = if blocked_subgraphs.is_empty() {
            DemandControlResultCode::CostOk
        } else {
            DemandControlResultCode::CostEstimatedTooExpensive
        };

        metrics.demand_control.record_estimated_cost(
            estimation.estimated_cost,
            &estimated_result,
            operation_name,
        );

        demand_control_span.record_result(
            estimation.estimated_cost,
            blocked_subgraphs.len(),
            &estimated_result,
        );

        // Compile per-service actual-cost formulas for BySubgraph mode
        let is_by_subgraph = config
            .actual_cost
            .as_ref()
            .map(|a| {
                matches!(
                    a.mode,
                    hive_router_config::demand_control::DemandControlActualCostMode::BySubgraph
                )
            })
            .unwrap_or(false);

        let actual_cost_plan_by_service = if is_by_subgraph {
            let mut by_service: AHashMap<String, Arc<Vec<ActualCostPlanNode>>> = AHashMap::new();
            collect_actual_cost_plan_nodes(&compiled_plan.root, &mut by_service);
            Some(by_service)
        } else {
            None
        };

        Ok(Some(DemandControlExecutionContext {
            max_cost: config.max_cost,
            evaluation: estimation,
            blocked_subgraphs,
            actual_cost_mode: config.actual_cost.as_ref().map(|actual| actual.mode),
            result_code: estimated_result,
            metrics_recorder: metrics.demand_control.recorder(),
            include_extension_metadata: config.include_extension_metadata.unwrap_or(false),
            formula_cache_hit,
            estimated_formula_by_subgraph: compiled_plan.formula_by_subgraph.clone(),
            actual_cost_plan_by_service,
        }))
    }
    .instrument(demand_control_span.clone())
    .await
}

fn collect_estimated_formulas(node: &FormulaPlanNode, formulas: &mut AHashMap<String, CostExpr>) {
    match node {
        FormulaPlanNode::Empty => {}
        FormulaPlanNode::Fetch(fetch) => {
            formulas
                .entry(fetch.service_name.clone())
                .and_modify(|expr| {
                    *expr = CostExpr::add_nonzero(vec![expr.clone(), fetch.estimated_expr.clone()]);
                })
                .or_insert_with(|| fetch.estimated_expr.clone());
        }
        FormulaPlanNode::Sequence(nodes) | FormulaPlanNode::Parallel(nodes) => {
            for child in nodes {
                collect_estimated_formulas(child, formulas);
            }
        }
        FormulaPlanNode::Condition {
            if_clause,
            else_clause,
            ..
        } => {
            if let Some(c) = if_clause.as_deref() {
                collect_estimated_formulas(c, formulas);
            }
            if let Some(c) = else_clause.as_deref() {
                collect_estimated_formulas(c, formulas);
            }
        }
        FormulaPlanNode::Subscription(node) => collect_estimated_formulas(node, formulas),
        FormulaPlanNode::Defer { primary, deferred } => {
            if let Some(p) = primary.as_deref() {
                collect_estimated_formulas(p, formulas);
            }
            for child in deferred {
                collect_estimated_formulas(child, formulas);
            }
        }
    }
}

fn collect_actual_cost_plan_nodes(
    node: &FormulaPlanNode,
    by_service: &mut AHashMap<String, Arc<Vec<ActualCostPlanNode>>>,
) {
    match node {
        FormulaPlanNode::Empty => {}
        FormulaPlanNode::Fetch(fetch) => {
            by_service
                .entry(fetch.service_name.clone())
                .or_insert_with(|| fetch.actual_nodes.clone());
        }
        FormulaPlanNode::Sequence(nodes) | FormulaPlanNode::Parallel(nodes) => {
            for child in nodes {
                collect_actual_cost_plan_nodes(child, by_service);
            }
        }
        FormulaPlanNode::Condition {
            if_clause,
            else_clause,
            ..
        } => {
            if let Some(c) = if_clause.as_deref() {
                collect_actual_cost_plan_nodes(c, by_service);
            }
            if let Some(c) = else_clause.as_deref() {
                collect_actual_cost_plan_nodes(c, by_service);
            }
        }
        FormulaPlanNode::Subscription(node) => collect_actual_cost_plan_nodes(node, by_service),
        FormulaPlanNode::Defer { primary, deferred } => {
            if let Some(p) = primary.as_deref() {
                collect_actual_cost_plan_nodes(p, by_service);
            }
            for child in deferred {
                collect_actual_cost_plan_nodes(child, by_service);
            }
        }
    }
}

fn default_list_size_for_subgraph(
    config: &hive_router_config::demand_control::DemandControlConfig,
    subgraph_name: &str,
) -> usize {
    config
        .subgraph
        .as_ref()
        .and_then(|subgraph_config| {
            subgraph_config
                .subgraphs
                .get(subgraph_name)
                .and_then(|cfg| cfg.list_size)
                .or_else(|| subgraph_config.all.as_ref().and_then(|cfg| cfg.list_size))
        })
        .or(config.list_size)
        .unwrap_or(0)
}

fn blocked_subgraphs(
    config: &hive_router_config::demand_control::DemandControlConfig,
    evaluation: &DemandControlEvaluation,
) -> AHashSet<String> {
    let mut blocked = AHashSet::new();

    let Some(subgraph_config) = &config.subgraph else {
        return blocked;
    };

    let inherited_max = subgraph_config.all.as_ref().and_then(|cfg| cfg.max_cost);

    for (subgraph, estimated_cost) in &evaluation.per_subgraph {
        let specific_max = subgraph_config
            .subgraphs
            .get(subgraph.as_str())
            .and_then(|cfg| cfg.max_cost);
        let max = specific_max.or(inherited_max);

        if let Some(limit) = max {
            if *estimated_cost > limit {
                blocked.insert(subgraph.clone());
            }
        }
    }

    blocked
}

fn compile_demand_control_plan(
    query_plan: &QueryPlan,
    supergraph_state: &SupergraphState,
    config: &hive_router_config::demand_control::DemandControlConfig,
) -> DemandControlFormulaPlan {
    let root = query_plan
        .node
        .as_ref()
        .map(|node| compile_plan_node(node, supergraph_state, config))
        .unwrap_or(FormulaPlanNode::Empty);
    let mut expr_by_subgraph = AHashMap::new();
    collect_estimated_formulas(&root, &mut expr_by_subgraph);
    let formula_by_subgraph: AHashMap<String, String> = expr_by_subgraph
        .into_iter()
        .map(|(service, expr)| (service, expr.to_string()))
        .collect();

    DemandControlFormulaPlan {
        root,
        formula_by_subgraph: Arc::new(formula_by_subgraph),
    }
}

fn compile_plan_node(
    node: &PlanNode,
    supergraph_state: &SupergraphState,
    config: &hive_router_config::demand_control::DemandControlConfig,
) -> FormulaPlanNode {
    match node {
        PlanNode::Fetch(fetch_node) => {
            let default_list_size =
                default_list_size_for_subgraph(config, &fetch_node.service_name);
            let root_type = supergraph_state.root_type_name(fetch_node.operation_kind.as_ref());
            FormulaPlanNode::Fetch(FormulaFetchNode {
                service_name: fetch_node.service_name.clone(),
                estimated_expr: compile_operation_expr(
                    &fetch_node.operation.document.operation,
                    &fetch_node.operation.document.fragments,
                    root_type,
                    fetch_node.operation_kind.as_ref(),
                    supergraph_state,
                    default_list_size,
                ),
                actual_nodes: Arc::new(compile_actual_cost_plan_nodes(
                    &fetch_node.operation.document.operation.selection_set,
                    root_type,
                    supergraph_state,
                )),
            })
        }
        PlanNode::BatchFetch(batch_fetch_node) => {
            let default_list_size =
                default_list_size_for_subgraph(config, &batch_fetch_node.service_name);
            let root_type =
                supergraph_state.root_type_name(batch_fetch_node.operation_kind.as_ref());
            FormulaPlanNode::Fetch(FormulaFetchNode {
                service_name: batch_fetch_node.service_name.clone(),
                estimated_expr: compile_operation_expr(
                    &batch_fetch_node.operation.document.operation,
                    &batch_fetch_node.operation.document.fragments,
                    root_type,
                    batch_fetch_node.operation_kind.as_ref(),
                    supergraph_state,
                    default_list_size,
                ),
                actual_nodes: Arc::new(compile_actual_cost_plan_nodes(
                    &batch_fetch_node.operation.document.operation.selection_set,
                    root_type,
                    supergraph_state,
                )),
            })
        }
        PlanNode::Flatten(flatten) => compile_plan_node(&flatten.node, supergraph_state, config),
        PlanNode::Sequence(sequence) => FormulaPlanNode::Sequence(
            sequence
                .nodes
                .iter()
                .map(|child| compile_plan_node(child, supergraph_state, config))
                .collect(),
        ),
        PlanNode::Parallel(parallel) => FormulaPlanNode::Parallel(
            parallel
                .nodes
                .iter()
                .map(|child| compile_plan_node(child, supergraph_state, config))
                .collect(),
        ),
        PlanNode::Condition(condition) => FormulaPlanNode::Condition {
            condition: condition.condition.clone(),
            if_clause: condition
                .if_clause
                .as_ref()
                .map(|node| Box::new(compile_plan_node(node, supergraph_state, config))),
            else_clause: condition
                .else_clause
                .as_ref()
                .map(|node| Box::new(compile_plan_node(node, supergraph_state, config))),
        },
        PlanNode::Subscription(subscription) => {
            let default_list_size =
                default_list_size_for_subgraph(config, &subscription.primary.service_name);
            let root_type =
                supergraph_state.root_type_name(subscription.primary.operation_kind.as_ref());
            FormulaPlanNode::Subscription(Box::new(FormulaPlanNode::Fetch(FormulaFetchNode {
                service_name: subscription.primary.service_name.clone(),
                estimated_expr: compile_operation_expr(
                    &subscription.primary.operation.document.operation,
                    &subscription.primary.operation.document.fragments,
                    root_type,
                    subscription.primary.operation_kind.as_ref(),
                    supergraph_state,
                    default_list_size,
                ),
                actual_nodes: Arc::new(compile_actual_cost_plan_nodes(
                    &subscription
                        .primary
                        .operation
                        .document
                        .operation
                        .selection_set,
                    root_type,
                    supergraph_state,
                )),
            })))
        }
        PlanNode::Defer(defer) => FormulaPlanNode::Defer {
            primary: defer
                .primary
                .node
                .as_ref()
                .map(|node| Box::new(compile_plan_node(node, supergraph_state, config))),
            deferred: defer
                .deferred
                .iter()
                .filter_map(|node| node.node.as_ref())
                .map(|node| compile_plan_node(node, supergraph_state, config))
                .collect(),
        },
    }
}

// ── Evaluate phase: plan ─────────────────────────────────────────────────────

fn evaluate_compiled_plan(
    compiled_plan: &DemandControlFormulaPlan,
    supergraph_state: &SupergraphState,
    variable_payload: &CoerceVariablesPayload,
) -> DemandControlEvaluation {
    let mut evaluation = DemandControlEvaluation::default();
    evaluate_compiled_plan_node(
        &compiled_plan.root,
        supergraph_state,
        variable_payload,
        &mut evaluation,
    );
    evaluation
}

fn evaluate_compiled_plan_node(
    node: &FormulaPlanNode,
    supergraph_state: &SupergraphState,
    variable_payload: &CoerceVariablesPayload,
    out: &mut DemandControlEvaluation,
) {
    match node {
        FormulaPlanNode::Empty => {}
        FormulaPlanNode::Fetch(fetch_node) => {
            let cost = eval_cost_expr(
                &fetch_node.estimated_expr,
                supergraph_state,
                variable_payload,
            );
            out.estimated_cost = out.estimated_cost.saturating_add(cost);
            if let Some(subgraph_cost) = out.per_subgraph.get_mut(&fetch_node.service_name) {
                *subgraph_cost = subgraph_cost.saturating_add(cost);
            } else {
                out.per_subgraph
                    .insert(fetch_node.service_name.clone(), cost);
            }
        }
        FormulaPlanNode::Sequence(nodes) | FormulaPlanNode::Parallel(nodes) => {
            for child in nodes {
                evaluate_compiled_plan_node(child, supergraph_state, variable_payload, out);
            }
        }
        FormulaPlanNode::Condition {
            condition,
            if_clause,
            else_clause,
        } => {
            let branch = if variable_payload.variable_equals_true(condition) {
                if_clause.as_deref()
            } else {
                else_clause.as_deref()
            };

            if let Some(child) = branch {
                evaluate_compiled_plan_node(child, supergraph_state, variable_payload, out);
            }
        }
        FormulaPlanNode::Subscription(node) => {
            evaluate_compiled_plan_node(node, supergraph_state, variable_payload, out);
        }
        FormulaPlanNode::Defer { primary, deferred } => {
            if let Some(primary) = primary.as_deref() {
                evaluate_compiled_plan_node(primary, supergraph_state, variable_payload, out);
            }
            for child in deferred {
                evaluate_compiled_plan_node(child, supergraph_state, variable_payload, out);
            }
        }
    }
}

// ── Evaluate phase: CostExpr ─────────────────────────────────────────────────

/// Evaluate a CostExpr to a u64 cost.
/// Pure arithmetic + variable lookups only, so zero schema traversal at request time.
fn eval_cost_expr(
    expr: &CostExpr,
    supergraph_state: &SupergraphState,
    variable_payload: &CoerceVariablesPayload,
) -> u64 {
    match expr {
        CostExpr::Const(n) => *n,
        CostExpr::Add(exprs) => exprs
            .iter()
            .map(|e| eval_cost_expr(e, supergraph_state, variable_payload))
            .fold(0u64, |acc, v| acc.saturating_add(v)),
        CostExpr::Mul(lhs, rhs) => eval_cost_expr(lhs, supergraph_state, variable_payload)
            .saturating_mul(eval_cost_expr(rhs, supergraph_state, variable_payload)),
        CostExpr::Cond {
            variable,
            if_true,
            if_false,
        } => {
            if variable_payload.variable_equals_true(variable) {
                eval_cost_expr(if_true, supergraph_state, variable_payload)
            } else {
                eval_cost_expr(if_false, supergraph_state, variable_payload)
            }
        }
        CostExpr::ListSize {
            args,
            require_one,
            default,
        } => {
            let values: Vec<u64> = args
                .iter()
                .filter_map(|(value, path)| resolve_integer_value(value, path, variable_payload))
                .collect();
            if *require_one {
                if values.len() == 1 {
                    values[0]
                } else {
                    *default as u64
                }
            } else {
                values.into_iter().max().unwrap_or(*default as u64)
            }
        }
        CostExpr::InputArgCost { value, value_type } => {
            estimate_input_value_cost(value, value_type, supergraph_state, variable_payload)
        }
    }
}

// ── Compile phase: estimated cost → CostExpr ─────────────────────────────────

fn compile_operation_expr(
    operation: &OperationDefinition,
    operation_fragments: &[FragmentDefinition],
    root_type_name: &str,
    operation_kind: Option<&OperationKind>,
    supergraph_state: &SupergraphState,
    default_list_size: usize,
) -> CostExpr {
    let base = match operation_kind {
        Some(OperationKind::Mutation) => 10u64,
        _ => 0u64,
    };
    let mut fragments_cache = AHashMap::new();
    let mut visited_fragments = AHashSet::new();
    let empty_overrides: SizeOverrides = Vec::new();
    let fields_expr = compile_selection_set_expr(
        &operation.selection_set,
        operation_fragments,
        root_type_name,
        supergraph_state,
        &mut fragments_cache,
        &mut visited_fragments,
        &empty_overrides,
        default_list_size,
    );
    if base > 0 {
        CostExpr::add_nonzero(vec![CostExpr::Const(base), fields_expr])
    } else {
        fields_expr
    }
}

#[allow(clippy::too_many_arguments)]
fn compile_selection_set_expr<'exec>(
    selection_set: &'exec SelectionSet,
    operation_fragments: &'exec [FragmentDefinition],
    parent_type_name: &'exec str,
    supergraph_state: &'exec SupergraphState,
    fragments_cache: &mut AHashMap<&'exec str, &'exec FragmentDefinition>,
    visited_fragments: &mut AHashSet<&'exec str>,
    inherited_overrides: &SizeOverrides,
    default_list_size: usize,
) -> CostExpr {
    let mut field_exprs = Vec::new();
    for item in &selection_set.items {
        match item {
            SelectionItem::Field(field) => {
                let field_name = field.name.as_str();
                let self_override = inherited_overrides
                    .iter()
                    .find(|(path, _)| path.len() == 1 && path[0] == field_name)
                    .map(|(_, expr)| expr.clone());
                let child_inherited: SizeOverrides = inherited_overrides
                    .iter()
                    .filter(|(path, _)| path.len() > 1 && path[0] == field_name)
                    .map(|(path, expr)| (path[1..].to_vec(), expr.clone()))
                    .collect();
                field_exprs.push(compile_field_expr(
                    field,
                    operation_fragments,
                    parent_type_name,
                    supergraph_state,
                    fragments_cache,
                    visited_fragments,
                    self_override,
                    child_inherited,
                    default_list_size,
                ));
            }
            SelectionItem::InlineFragment(fragment) => {
                field_exprs.push(compile_selection_set_expr(
                    &fragment.selections,
                    operation_fragments,
                    fragment.type_condition.as_str(),
                    supergraph_state,
                    fragments_cache,
                    visited_fragments,
                    inherited_overrides,
                    default_list_size,
                ));
            }
            SelectionItem::FragmentSpread(fragment_name) => {
                if visited_fragments.contains(fragment_name.as_str()) {
                    continue;
                }
                if fragments_cache.is_empty() {
                    for fragment in operation_fragments {
                        fragments_cache.insert(fragment.name.as_str(), fragment);
                    }
                }
                let Some(fragment_def) = fragments_cache.get(fragment_name.as_str()) else {
                    continue;
                };
                visited_fragments.insert(fragment_name.as_str());
                field_exprs.push(compile_selection_set_expr(
                    &fragment_def.selection_set,
                    operation_fragments,
                    fragment_def.type_condition.as_str(),
                    supergraph_state,
                    fragments_cache,
                    visited_fragments,
                    inherited_overrides,
                    default_list_size,
                ));
                visited_fragments.remove(fragment_name.as_str());
            }
        }
    }
    CostExpr::add_nonzero(field_exprs)
}

#[allow(clippy::too_many_arguments)]
fn compile_field_expr<'exec>(
    field: &'exec FieldSelection,
    operation_fragments: &'exec [FragmentDefinition],
    parent_type_name: &'exec str,
    supergraph_state: &'exec SupergraphState,
    fragments_cache: &mut AHashMap<&'exec str, &'exec FragmentDefinition>,
    visited_fragments: &mut AHashSet<&'exec str>,
    override_list_size: Option<CostExpr>,
    extra_child_overrides: SizeOverrides,
    default_list_size: usize,
) -> CostExpr {
    let field_def = supergraph_state
        .definitions
        .get(parent_type_name)
        .and_then(|def| def.fields().get(field.name.as_str()));

    let mut base_parts: Vec<CostExpr> = Vec::new();
    let mut all_child_overrides = extra_child_overrides;
    let mut own_list_size_expr: Option<CostExpr> = override_list_size;

    let (return_type_name, _field_type_is_list) = if let Some(def) = field_def {
        let mut base_cost = def.cost.as_ref().map(|c| c.weight).unwrap_or(0);
        if let Some(arguments) = &field.arguments {
            for key in arguments.keys() {
                if let Some(cost) = def.cost_by_arguments.get(key) {
                    base_cost = base_cost.saturating_add(cost.weight);
                }
                if let Some(arg_type) = def.argument_types.get(key) {
                    if let Some(arg_value) = arguments.get_argument(key) {
                        base_parts.push(CostExpr::InputArgCost {
                            value: arg_value.clone(),
                            value_type: arg_type.clone(),
                        });
                    }
                }
            }
        }
        if base_cost > 0 {
            base_parts.insert(0, CostExpr::Const(base_cost));
        }
        if let Some(list_size_directive) = &def.list_size {
            let size_expr = compile_list_size_expr(list_size_directive, field, default_list_size);
            if let Some(sized_fields) = &list_size_directive.sized_fields {
                for path in sized_fields {
                    all_child_overrides.push((path.clone(), size_expr.clone()));
                }
            } else if def.field_type.is_list() && own_list_size_expr.is_none() {
                own_list_size_expr = Some(size_expr);
            }
        }
        if own_list_size_expr.is_none() && def.field_type.is_list() {
            own_list_size_expr = Some(CostExpr::Const(default_list_size as u64));
        }
        (def.field_type.inner_type(), def.field_type.is_list())
    } else {
        (parent_type_name, false)
    };

    let return_type_cost = type_cost(supergraph_state, return_type_name);
    let children_expr = compile_selection_set_expr(
        &field.selections,
        operation_fragments,
        return_type_name,
        supergraph_state,
        fragments_cache,
        visited_fragments,
        &all_child_overrides,
        default_list_size,
    );

    let field_cost_expr = if let Some(list_size) = own_list_size_expr {
        // base + list_size × (type_cost + child_cost)
        let mut per_item = Vec::new();
        if return_type_cost > 0 {
            per_item.push(CostExpr::Const(return_type_cost));
        }
        if !matches!(children_expr, CostExpr::Const(0)) {
            per_item.push(children_expr);
        }
        let list_cost = CostExpr::mul(list_size, CostExpr::add_nonzero(per_item));
        CostExpr::add_nonzero(vec![CostExpr::add_nonzero(base_parts), list_cost])
    } else {
        let mut parts = base_parts;
        if return_type_cost > 0 {
            parts.push(CostExpr::Const(return_type_cost));
        }
        if !matches!(children_expr, CostExpr::Const(0)) {
            parts.push(children_expr);
        }
        CostExpr::add_nonzero(parts)
    };

    // Respect GraphQL semantics when both are present:
    // include iff (@include == true) AND (@skip != true).
    let mut expr = field_cost_expr;
    if let Some(include_if) = &field.include_if {
        expr = CostExpr::Cond {
            variable: include_if.clone(),
            if_true: Box::new(expr),
            if_false: Box::new(CostExpr::Const(0)),
        };
    }
    if let Some(skip_if) = &field.skip_if {
        expr = CostExpr::Cond {
            variable: skip_if.clone(),
            if_true: Box::new(CostExpr::Const(0)),
            if_false: Box::new(expr),
        };
    }
    expr
}

fn compile_list_size_expr(
    directive: &hive_router_query_planner::federation_spec::demand_control::ListSizeDirective,
    field: &FieldSelection,
    default_list_size: usize,
) -> CostExpr {
    if let Some(assumed_size) = directive.assumed_size {
        return CostExpr::Const(assumed_size as u64);
    }
    let Some(slicing_arguments) = &directive.slicing_arguments else {
        return CostExpr::Const(default_list_size as u64);
    };
    let Some(arguments) = &field.arguments else {
        return CostExpr::Const(default_list_size as u64);
    };
    let mut args = Vec::new();
    for segments in slicing_arguments {
        if let Some(root_value) = arguments.get_argument(&segments[0]) {
            args.push((root_value.clone(), segments[1..].to_vec()));
        }
    }
    if args.is_empty() {
        return CostExpr::Const(default_list_size as u64);
    }
    CostExpr::ListSize {
        args,
        require_one: directive.require_one_slicing_argument,
        default: default_list_size,
    }
}

// ── Compile phase: actual cost → ActualCostPlanNode ─────────────────────────

fn compile_actual_cost_plan_nodes<'exec>(
    selection_set: &'exec SelectionSet,
    parent_type_name: &'exec str,
    supergraph_state: &'exec SupergraphState,
) -> Vec<ActualCostPlanNode> {
    let mut nodes = Vec::new();
    for item in &selection_set.items {
        match item {
            SelectionItem::Field(field) => {
                nodes.push(ActualCostPlanNode::Field(compile_actual_cost_plan_field(
                    field,
                    parent_type_name,
                    supergraph_state,
                )));
            }
            SelectionItem::InlineFragment(fragment) => {
                nodes.push(ActualCostPlanNode::InlineFragment {
                    type_condition: fragment.type_condition.clone(),
                    children: compile_actual_cost_plan_nodes(
                        &fragment.selections,
                        fragment.type_condition.as_str(),
                        supergraph_state,
                    ),
                });
            }
            SelectionItem::FragmentSpread(_) => {
                // Subgraph fetch operations have fragment spreads inlined
            }
        }
    }
    nodes
}

fn compile_actual_cost_plan_field<'exec>(
    field: &'exec FieldSelection,
    parent_type_name: &'exec str,
    supergraph_state: &'exec SupergraphState,
) -> ActualCostPlanField {
    let field_def = supergraph_state
        .definitions
        .get(parent_type_name)
        .and_then(|def| def.fields().get(field.name.as_str()));

    let (return_type_name, is_list, base_cost) = if let Some(def) = field_def {
        let mut base = def.cost.as_ref().map(|c| c.weight).unwrap_or(0);
        if let Some(args) = &field.arguments {
            for key in args.keys() {
                if let Some(cost) = def.cost_by_arguments.get(key) {
                    base = base.saturating_add(cost.weight);
                }
            }
        }
        (def.field_type.inner_type(), def.field_type.is_list(), base)
    } else {
        // `_entities` is a Federation synthetic field that always returns a list
        let is_list = field.name == "_entities";
        (parent_type_name, is_list, 0u64)
    };

    ActualCostPlanField {
        response_key: field.selection_identifier().to_string(),
        skip_if: field.skip_if.clone(),
        include_if: field.include_if.clone(),
        base_cost,
        is_list,
        per_item_cost: type_cost(supergraph_state, return_type_name),
        children: compile_actual_cost_plan_nodes(
            &field.selections,
            return_type_name,
            supergraph_state,
        ),
    }
}

fn estimate_input_value_cost(
    value: &AstValue,
    value_type: &TypeNode,
    supergraph_state: &SupergraphState,
    variable_payload: &CoerceVariablesPayload,
) -> u64 {
    match value {
        AstValue::Variable(var_name) => variable_payload
            .variables_map
            .as_ref()
            .and_then(|variables| variables.get(var_name))
            .map(|json_value| {
                estimate_input_json_value_cost(json_value, value_type, supergraph_state)
            })
            .unwrap_or(0),
        AstValue::Object(object) => estimate_input_object_cost(
            object.iter().map(|(key, value)| (key.as_str(), value)),
            value_type,
            supergraph_state,
            |nested_value, nested_type| {
                estimate_input_value_cost(
                    nested_value,
                    nested_type,
                    supergraph_state,
                    variable_payload,
                )
            },
        ),
        AstValue::List(values) => match unwrap_non_null(value_type) {
            TypeNode::List(inner_type) => values
                .iter()
                .map(|item| {
                    estimate_input_value_cost(item, inner_type, supergraph_state, variable_payload)
                })
                .sum(),
            _ => 0,
        },
        AstValue::Null => 0,
        _ => 0,
    }
}

fn estimate_input_json_value_cost(
    value: &Value,
    value_type: &TypeNode,
    supergraph_state: &SupergraphState,
) -> u64 {
    if value.is_null() {
        return 0;
    }

    if let Some(array) = value.as_array() {
        return match unwrap_non_null(value_type) {
            TypeNode::List(inner_type) => array
                .iter()
                .map(|item| estimate_input_json_value_cost(item, inner_type, supergraph_state))
                .sum(),
            _ => 0,
        };
    }

    if let Some(object) = value.as_object() {
        return estimate_input_object_cost(
            object.iter(),
            value_type,
            supergraph_state,
            |nested_value, nested_type| {
                estimate_input_json_value_cost(nested_value, nested_type, supergraph_state)
            },
        );
    }

    0
}

fn estimate_input_object_cost<'a, V, I, F>(
    fields: I,
    value_type: &TypeNode,
    supergraph_state: &SupergraphState,
    nested_cost: F,
) -> u64
where
    V: 'a,
    I: Iterator<Item = (&'a str, &'a V)>,
    F: Fn(&V, &TypeNode) -> u64,
{
    let TypeNode::Named(type_name) = unwrap_non_null(value_type) else {
        return 0;
    };

    let Some(SupergraphDefinition::InputObject(input_object)) =
        supergraph_state.definitions.get(type_name)
    else {
        return 0;
    };

    fields
        .filter_map(|(field_name, field_value)| {
            let input_field = input_object.fields.get(field_name)?;
            let field_cost = input_field
                .cost
                .as_ref()
                .map(|cost| cost.weight)
                .unwrap_or(0);
            Some(field_cost.saturating_add(nested_cost(field_value, &input_field.field_type)))
        })
        .sum()
}

fn unwrap_non_null(value_type: &TypeNode) -> &TypeNode {
    match value_type {
        TypeNode::NonNull(inner) => unwrap_non_null(inner),
        _ => value_type,
    }
}

fn resolve_integer_value(
    value: &AstValue,
    path: &[String],
    variable_payload: &CoerceVariablesPayload,
) -> Option<u64> {
    match value {
        AstValue::Int(v) => {
            if path.is_empty() {
                Some((*v).max(0) as u64)
            } else {
                None
            }
        }
        AstValue::Variable(var_name) => {
            let value = variable_payload
                .variables_map
                .as_ref()
                .and_then(|variables| variables.get(var_name))?;
            resolve_integer_from_json_value(value, path)
        }
        AstValue::Object(object) => {
            let (head, tail) = path.split_first()?;

            let nested_value = object.get(head)?;
            resolve_integer_value(nested_value, tail, variable_payload)
        }
        _ => None,
    }
}

fn resolve_integer_from_json_value(value: &Value, path: &[String]) -> Option<u64> {
    if let Some((head, tail)) = path.split_first() {
        let object = value.as_object()?;
        let nested = object.get(head)?;
        return resolve_integer_from_json_value(nested, tail);
    }

    if let Some(v) = value.as_u64() {
        return Some(v);
    }

    if let Some(v) = value.as_i64() {
        return Some(v.max(0) as u64);
    }

    None
}

fn type_cost(supergraph_state: &SupergraphState, type_name: &str) -> u64 {
    let Some(definition) = supergraph_state.definitions.get(type_name) else {
        return 0;
    };

    match definition {
        SupergraphDefinition::Object(def) => def.cost.as_ref().map(|cost| cost.weight).unwrap_or(1),
        SupergraphDefinition::Interface(_) | SupergraphDefinition::Union(_) => 1,
        SupergraphDefinition::Enum(def) => def.cost.as_ref().map(|cost| cost.weight).unwrap_or(0),
        SupergraphDefinition::Scalar(def) => def.cost.as_ref().map(|cost| cost.weight).unwrap_or(0),
        SupergraphDefinition::InputObject(_) => 0,
    }
}
