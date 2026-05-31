use ahash::{HashMap as AHashMap, HashMapExt, HashSet as AHashSet, HashSetExt};

use hive_router_plan_executor::execution::demand_control::CompiledActualCostPlan;
use hive_router_plan_executor::execution::demand_control::DemandControlEvaluation;
use hive_router_plan_executor::execution::plan::CoerceVariablesPayload;
use hive_router_query_planner::federation_spec::demand_control::ListSizeDirective;
use hive_router_query_planner::{
    ast::{
        fragment::FragmentDefinition,
        operation::OperationDefinition,
        selection_item::SelectionItem,
        selection_set::{FieldSelection, SelectionSet},
        value::Value as AstValue,
    },
    state::supergraph_state::{OperationKind, SupergraphDefinition, SupergraphState, TypeNode},
};
use sonic_rs::{JsonContainerTrait, JsonValueTrait, Value};

use std::{collections::BTreeMap, fmt, sync::Arc};

use crate::pipeline::error::PipelineError;

// ── CostExpr compilation as a mathematical formula for estimated cost ───────────────────────

/// Size overrides threaded from a parent @listSize(sizedFields:[…]) down to children.
/// Each entry is (remaining path from the current selection, size expression).
type SizeOverrides = Vec<(Vec<String>, CostExpr)>;

/// A mathematical cost expression compiled once per query shape.
/// Evaluated with only variable lookups, so no schema traversal at request time.
#[derive(Clone, Debug)]
pub(crate) enum CostExpr {
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
        field_name: String,
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
        let mut final_exprs = Vec::with_capacity(exprs.len());
        let mut const_total = 0u64;
        for expr in exprs {
            match expr {
                Self::Const(val) => {
                    const_total = const_total.saturating_add(val);
                }
                _ => {
                    final_exprs.push(expr);
                }
            }
        }
        if const_total > 0 {
            final_exprs.push(Self::Const(const_total));
        }
        match final_exprs.len() {
            0 => Self::Const(0),
            1 => final_exprs.into_iter().next().unwrap(),
            _ => Self::Add(final_exprs),
        }
    }

    /// Build a Mul with constant-folding.
    fn mul(lhs: Self, rhs: Self) -> Self {
        match (&lhs, &rhs) {
            (Self::Const(0), _) | (_, Self::Const(0)) => Self::Const(0),
            (Self::Const(1), _) => rhs,
            (_, Self::Const(1)) => lhs,
            (Self::Const(a), Self::Const(b)) => Self::Const(a.saturating_mul(*b)),
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
                field_name: _,
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
    pub(crate) root: FormulaPlanNode,
    pub(crate) formula_by_subgraph: Arc<BTreeMap<String, String>>,
    pub(crate) actual_cost_plan: Arc<CompiledActualCostPlan>,
}

pub(crate) enum FormulaPlanNode {
    Fetch(FormulaFetchNode),
    Aggregate(Vec<FormulaPlanNode>),
    Condition {
        condition: String,
        if_clause: Option<Box<FormulaPlanNode>>,
        else_clause: Option<Box<FormulaPlanNode>>,
    },
}

pub(crate) struct FormulaFetchNode {
    pub(crate) service_name: String,
    /// Mathematical formula: pure arithmetic on variable lookups.
    pub(crate) estimated_expr: CostExpr,
}

pub(crate) fn collect_estimated_formulas(
    node: &FormulaPlanNode,
    formulas: &mut AHashMap<String, CostExpr>,
) {
    match node {
        FormulaPlanNode::Fetch(fetch) => {
            formulas
                .entry(fetch.service_name.clone())
                .and_modify(|expr| {
                    *expr = CostExpr::add_nonzero(vec![expr.clone(), fetch.estimated_expr.clone()]);
                })
                .or_insert_with(|| fetch.estimated_expr.clone());
        }
        FormulaPlanNode::Aggregate(nodes) => {
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
    }
}

// ── Evaluate phase: plan ─────────────────────────────────────────────────────

pub(crate) fn evaluate_formula_plan(
    formula_plan: &DemandControlFormulaPlan,
    supergraph_state: &SupergraphState,
    variable_payload: &CoerceVariablesPayload,
) -> Result<DemandControlEvaluation, PipelineError> {
    let mut per_subgraph = BTreeMap::new();
    let mut estimation = 0u64;
    evaluate_formula_plan_node(
        &formula_plan.root,
        supergraph_state,
        variable_payload,
        &mut per_subgraph,
        &mut estimation,
    )?;
    Ok(DemandControlEvaluation {
        estimated_cost: estimation,
        per_subgraph: Arc::new(per_subgraph),
    })
}

fn evaluate_formula_plan_node(
    node: &FormulaPlanNode,
    supergraph_state: &SupergraphState,
    variable_payload: &CoerceVariablesPayload,
    per_subgraph: &mut BTreeMap<String, u64>,
    estimated_cost: &mut u64,
) -> Result<(), PipelineError> {
    match node {
        FormulaPlanNode::Fetch(fetch_node) => {
            let cost = eval_cost_expr(
                &fetch_node.estimated_expr,
                supergraph_state,
                variable_payload,
            )?;
            *estimated_cost = estimated_cost.saturating_add(cost);
            if let Some(subgraph_cost) = per_subgraph.get_mut(&fetch_node.service_name) {
                *subgraph_cost = subgraph_cost.saturating_add(cost);
            } else {
                per_subgraph.insert(fetch_node.service_name.clone(), cost);
            }
        }
        FormulaPlanNode::Aggregate(nodes) => {
            for child in nodes {
                evaluate_formula_plan_node(
                    child,
                    supergraph_state,
                    variable_payload,
                    per_subgraph,
                    estimated_cost,
                )?;
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
                evaluate_formula_plan_node(
                    child,
                    supergraph_state,
                    variable_payload,
                    per_subgraph,
                    estimated_cost,
                )?;
            }
        }
    }

    Ok(())
}

// ── Evaluate phase: CostExpr ─────────────────────────────────────────────────

/// Evaluate a CostExpr to a u64 cost.
/// Pure arithmetic + variable lookups only, so zero schema traversal at request time.
fn eval_cost_expr(
    expr: &CostExpr,
    supergraph_state: &SupergraphState,
    variable_payload: &CoerceVariablesPayload,
) -> Result<u64, PipelineError> {
    match expr {
        CostExpr::Const(n) => Ok(*n),
        CostExpr::Add(exprs) => {
            let mut total = 0u64;
            for e in exprs {
                total =
                    total.saturating_add(eval_cost_expr(e, supergraph_state, variable_payload)?);
            }
            Ok(total)
        }
        CostExpr::Mul(lhs, rhs) => {
            let lhs = eval_cost_expr(lhs, supergraph_state, variable_payload)?;
            let rhs = eval_cost_expr(rhs, supergraph_state, variable_payload)?;
            Ok(lhs.saturating_mul(rhs))
        }
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
            field_name,
            args,
            require_one,
            default,
        } => {
            if *require_one {
                let mut resolved_count = 0usize;
                let mut only_value = 0u64;
                for (value, path) in args {
                    if let Some(resolved) = resolve_integer_value(value, path, variable_payload) {
                        resolved_count += 1;
                        only_value = resolved;
                    }
                }

                if resolved_count == 1 {
                    Ok(only_value)
                } else {
                    Err(PipelineError::CostInvalidSlicingArguments {
                        field_name: field_name.clone(),
                        found: resolved_count,
                    })
                }
            } else {
                let mut max_value: Option<u64> = None;
                for (value, path) in args {
                    if let Some(resolved) = resolve_integer_value(value, path, variable_payload) {
                        max_value = Some(match max_value {
                            Some(current_max) => current_max.max(resolved),
                            None => resolved,
                        });
                    }
                }

                Ok(max_value.unwrap_or(*default as u64))
            }
        }
        CostExpr::InputArgCost { value, value_type } => Ok(estimate_input_value_cost(
            value,
            value_type,
            supergraph_state,
            variable_payload,
        )),
    }
}

// ── Compile phase: estimated cost → CostExpr ─────────────────────────────────

pub(crate) fn compile_cost_expr_for_operation(
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
    let fields_expr = compile_cost_expr_for_selection_set(
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
fn compile_cost_expr_for_selection_set<'exec>(
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
                field_exprs.push(compile_cost_expr_for_field_selection(
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
                field_exprs.push(compile_cost_expr_for_selection_set(
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
                field_exprs.push(compile_cost_expr_for_selection_set(
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
fn compile_cost_expr_for_field_selection<'exec>(
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
    if field.name == "__typename" {
        return apply_field_conditions(field, CostExpr::Const(0));
    }

    if field.name == "_entities" {
        return apply_field_conditions(
            field,
            compile_cost_expr_for_entities_field_selection(
                field,
                operation_fragments,
                parent_type_name,
                supergraph_state,
                fragments_cache,
                visited_fragments,
                default_list_size,
            ),
        );
    }

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
            let size_expr =
                compile_cost_expr_for_list_size(list_size_directive, field, default_list_size);
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
    let children_expr = compile_cost_expr_for_selection_set(
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
    apply_field_conditions(field, field_cost_expr)
}

fn compile_cost_expr_for_entities_field_selection<'exec>(
    field: &'exec FieldSelection,
    operation_fragments: &'exec [FragmentDefinition],
    parent_type_name: &'exec str,
    supergraph_state: &'exec SupergraphState,
    fragments_cache: &mut AHashMap<&'exec str, &'exec FragmentDefinition>,
    visited_fragments: &mut AHashSet<&'exec str>,
    default_list_size: usize,
) -> CostExpr {
    // The `representations` argument is injected by the query planner at runtime
    // and is not part of the user-provided variables payload, so it cannot be
    // resolved during estimated-cost evaluation. Conceptually the entity list
    // mirrors the parent list that produced those representations, so we
    // approximate its size with the configured default `list_size`. We always
    // count at least one entity, otherwise an `_entities` fetch coming from a
    // singleton parent (or a configuration with `list_size: 0`) would be
    // estimated as zero work.
    let entity_count = CostExpr::Const(default_list_size.max(1) as u64);

    let entity_type_cost = field
        .selections
        .items
        .iter()
        .filter_map(|item| match item {
            SelectionItem::InlineFragment(fragment) => Some(type_cost(
                supergraph_state,
                fragment.type_condition.as_str(),
            )),
            _ => None,
        })
        .max()
        .unwrap_or(0);

    let per_entity_children = compile_cost_expr_for_selection_set(
        &field.selections,
        operation_fragments,
        parent_type_name,
        supergraph_state,
        fragments_cache,
        visited_fragments,
        &Vec::new(),
        default_list_size,
    );

    let mut per_entity_parts = Vec::new();
    if entity_type_cost > 0 {
        per_entity_parts.push(CostExpr::Const(entity_type_cost));
    }
    if !matches!(per_entity_children, CostExpr::Const(0)) {
        per_entity_parts.push(per_entity_children);
    }

    CostExpr::mul(entity_count, CostExpr::add_nonzero(per_entity_parts))
}

fn apply_field_conditions(field: &FieldSelection, field_cost_expr: CostExpr) -> CostExpr {
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

fn compile_cost_expr_for_list_size(
    directive: &ListSizeDirective,
    field: &FieldSelection,
    default_list_size: usize,
) -> CostExpr {
    if let Some(assumed_size) = directive.assumed_size {
        return CostExpr::Const(assumed_size as u64);
    }
    let Some(slicing_arguments) = &directive.slicing_arguments else {
        return CostExpr::Const(default_list_size as u64);
    };
    let mut args = Vec::new();
    if let Some(arguments) = &field.arguments {
        for segments in slicing_arguments {
            if let Some(root_value) = arguments.get_argument(&segments[0]) {
                args.push((root_value.clone(), segments[1..].to_vec()));
            }
        }
    }
    // When `requireOneSlicingArgument` is set we must always evaluate at request
    // time so that the "exactly one slicing argument" rule can be enforced (0 or
    // 2+ resolved arguments → error), even when none of the slicing arguments are
    // present on the field. Without it, an empty `args` list can safely fall back
    // to the default list size.
    if !directive.require_one_slicing_argument && args.is_empty() {
        return CostExpr::Const(default_list_size as u64);
    }
    CostExpr::ListSize {
        field_name: field.name.clone(),
        args,
        require_one: directive.require_one_slicing_argument,
        default: default_list_size,
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
        .sum::<u64>()
        // Every input object instance contributes a default cost of 1 in
        // addition to any per-field `@cost(weight)` contributions, so an
        // input object without `@cost` directives still consumes budget
        // proportional to the depth/breadth of the values being passed in.
        .saturating_add(1)
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
        AstValue::List(items) => {
            if path.is_empty() {
                Some(items.len() as u64)
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

    if let Some(arr) = value.as_array() {
        return Some(arr.len() as u64);
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
