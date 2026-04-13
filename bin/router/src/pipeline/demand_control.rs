use std::collections::{HashMap, HashSet};

use hive_router_internal::telemetry::metrics::demand_control_metrics::DemandControlResultCode;
use hive_router_plan_executor::{
    execution::demand_control::{DemandControlEvaluation, DemandControlExecutionContext},
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
    planner::plan_nodes::{BatchFetchNode, ConditionNode, FetchNode, PlanNode, QueryPlan},
    state::supergraph_state::{OperationKind, SupergraphDefinition, SupergraphState, TypeNode},
};
use sonic_rs::{JsonContainerTrait, JsonValueTrait, Value};

use crate::{
    pipeline::{coerce_variables::CoerceVariablesPayload, error::PipelineError},
    shared_state::RouterSharedState,
};

pub fn evaluate_demand_control<'exec>(
    app_state: &'exec RouterSharedState,
    supergraph: &'exec SupergraphData,
    variable_payload: &'exec CoerceVariablesPayload,
    query_plan: &'exec QueryPlan,
    operation_name: Option<&'exec str>,
) -> Result<Option<DemandControlExecutionContext<'exec>>, PipelineError> {
    let Some(config) = &app_state.router_config.demand_control else {
        return Ok(None);
    };

    if !config.enabled {
        return Ok(None);
    }

    let evaluation = estimate_query_plan_cost(
        query_plan,
        &supergraph.planner.supergraph,
        variable_payload,
        config,
    );

    if let Some(max_cost) = config.max_cost {
        if evaluation.estimated_cost > max_cost {
            app_state
                .telemetry_context
                .metrics
                .demand_control
                .record_estimated_cost(
                    evaluation.estimated_cost,
                    &DemandControlResultCode::CostEstimatedTooExpensive,
                    operation_name,
                );
            return Err(PipelineError::CostEstimatedTooExpensive {
                estimated_cost: evaluation.estimated_cost,
                max_cost,
            });
        }
    }

    let blocked_subgraphs = blocked_subgraphs(config, &evaluation);
    let estimated_result = if blocked_subgraphs.is_empty() {
        DemandControlResultCode::CostOk
    } else {
        DemandControlResultCode::CostEstimatedTooExpensive
    };

    app_state
        .telemetry_context
        .metrics
        .demand_control
        .record_estimated_cost(evaluation.estimated_cost, &estimated_result, operation_name);

    Ok(Some(DemandControlExecutionContext {
        max_cost: config.max_cost,
        evaluation,
        blocked_subgraphs,
        operation_name,
        actual_cost_mode: config.actual_cost.as_ref().map(|actual| actual.mode),
        result_code: estimated_result,
        metrics_recorder: app_state
            .telemetry_context
            .metrics
            .demand_control
            .recorder(),
        include_extension_metadata: app_state
            .router_config
            .demand_control
            .as_ref()
            .and_then(|config| config.include_extension_metadata)
            .unwrap_or(false),
    }))
}

fn estimate_query_plan_cost<'exec>(
    query_plan: &'exec QueryPlan,
    supergraph_state: &'exec SupergraphState,
    variable_payload: &'exec CoerceVariablesPayload,
    config: &'exec hive_router_config::demand_control::DemandControlConfig,
) -> DemandControlEvaluation<'exec> {
    let mut evaluation: DemandControlEvaluation<'exec> = DemandControlEvaluation::default();
    if let Some(node) = &query_plan.node {
        let mut fragments = HashMap::new();
        estimate_plan_node_cost(
            node,
            supergraph_state,
            variable_payload,
            config,
            &mut fragments,
            &mut evaluation,
        );
    }
    evaluation
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

fn blocked_subgraphs<'exec>(
    config: &'exec hive_router_config::demand_control::DemandControlConfig,
    evaluation: &DemandControlEvaluation<'exec>,
) -> HashSet<&'exec str> {
    let mut blocked = HashSet::new();

    let Some(subgraph_config) = &config.subgraph else {
        return blocked;
    };

    for (subgraph, estimated_cost) in &evaluation.per_subgraph {
        let specific_max = subgraph_config
            .subgraphs
            .get(*subgraph)
            .and_then(|cfg| cfg.max_cost);
        let inherited_max = subgraph_config.all.as_ref().and_then(|cfg| cfg.max_cost);
        let max = specific_max.or(inherited_max);

        if let Some(limit) = max {
            if *estimated_cost > limit {
                blocked.insert(subgraph);
            }
        }
    }

    blocked
}

fn estimate_plan_node_cost<'exec>(
    node: &'exec PlanNode,
    supergraph_state: &'exec SupergraphState,
    variable_payload: &'exec CoerceVariablesPayload,
    config: &'exec hive_router_config::demand_control::DemandControlConfig,
    fragments: &mut HashMap<&'exec str, &'exec FragmentDefinition>,
    out: &mut DemandControlEvaluation<'exec>,
) {
    match node {
        PlanNode::Fetch(fetch) => {
            estimate_fetch_node_cost(
                fetch,
                supergraph_state,
                variable_payload,
                config,
                fragments,
                out,
            );
        }
        PlanNode::BatchFetch(fetch) => {
            estimate_batch_fetch_node_cost(
                fetch,
                supergraph_state,
                variable_payload,
                config,
                fragments,
                out,
            );
        }
        PlanNode::Flatten(flatten) => {
            estimate_plan_node_cost(
                &flatten.node,
                supergraph_state,
                variable_payload,
                config,
                fragments,
                out,
            );
        }
        PlanNode::Sequence(sequence) => {
            for child in &sequence.nodes {
                estimate_plan_node_cost(
                    child,
                    supergraph_state,
                    variable_payload,
                    config,
                    fragments,
                    out,
                );
            }
        }
        PlanNode::Parallel(parallel) => {
            for child in &parallel.nodes {
                estimate_plan_node_cost(
                    child,
                    supergraph_state,
                    variable_payload,
                    config,
                    fragments,
                    out,
                );
            }
        }
        PlanNode::Condition(condition) => {
            estimate_condition_node_cost(
                condition,
                supergraph_state,
                variable_payload,
                config,
                fragments,
                out,
            );
        }
        PlanNode::Subscription(subscription) => {
            estimate_plan_node_cost(
                &subscription.primary,
                supergraph_state,
                variable_payload,
                config,
                fragments,
                out,
            );
        }
        PlanNode::Defer(defer) => {
            if let Some(primary) = &defer.primary.node {
                estimate_plan_node_cost(
                    primary,
                    supergraph_state,
                    variable_payload,
                    config,
                    fragments,
                    out,
                );
            }

            for deferred in &defer.deferred {
                if let Some(child) = &deferred.node {
                    estimate_plan_node_cost(
                        child,
                        supergraph_state,
                        variable_payload,
                        config,
                        fragments,
                        out,
                    );
                }
            }
        }
    }
}

fn estimate_condition_node_cost<'exec>(
    condition: &'exec ConditionNode,
    supergraph_state: &'exec SupergraphState,
    variable_payload: &'exec CoerceVariablesPayload,
    config: &'exec hive_router_config::demand_control::DemandControlConfig,
    fragments: &mut HashMap<&'exec str, &'exec FragmentDefinition>,
    out: &mut DemandControlEvaluation<'exec>,
) {
    let condition_value = variable_payload.variable_equals_true(&condition.condition);

    if condition_value {
        if let Some(if_clause) = &condition.if_clause {
            estimate_plan_node_cost(
                if_clause,
                supergraph_state,
                variable_payload,
                config,
                fragments,
                out,
            );
        }
    } else if let Some(else_clause) = &condition.else_clause {
        estimate_plan_node_cost(
            else_clause,
            supergraph_state,
            variable_payload,
            config,
            fragments,
            out,
        );
    }
}

fn estimate_fetch_node_cost<'exec>(
    fetch: &'exec FetchNode,
    supergraph_state: &'exec SupergraphState,
    variable_payload: &'exec CoerceVariablesPayload,
    config: &'exec hive_router_config::demand_control::DemandControlConfig,
    fragments: &mut HashMap<&'exec str, &'exec FragmentDefinition>,
    out: &mut DemandControlEvaluation<'exec>,
) {
    let operation_kind = fetch
        .operation_kind
        .as_ref()
        .unwrap_or(&OperationKind::Query);
    let root_type_name = root_type_name_for_operation_kind(supergraph_state, operation_kind);
    let operation = &fetch.operation.document.operation;
    let default_list_size = default_list_size_for_subgraph(config, &fetch.service_name);
    let cost = estimate_operation_cost(
        operation,
        &fetch.operation.document.fragments,
        root_type_name,
        operation_kind,
        supergraph_state,
        variable_payload,
        default_list_size,
        fragments,
    );

    out.estimated_cost = out.estimated_cost.saturating_add(cost);
    let subgraph_cost = out.per_subgraph.entry(&fetch.service_name).or_insert(0);
    *subgraph_cost = subgraph_cost.saturating_add(cost);
}

fn estimate_batch_fetch_node_cost<'exec>(
    fetch: &'exec BatchFetchNode,
    supergraph_state: &'exec SupergraphState,
    variable_payload: &'exec CoerceVariablesPayload,
    config: &'exec hive_router_config::demand_control::DemandControlConfig,
    fragments: &mut HashMap<&'exec str, &'exec FragmentDefinition>,
    out: &mut DemandControlEvaluation<'exec>,
) {
    let operation_kind = fetch
        .operation_kind
        .as_ref()
        .unwrap_or(&OperationKind::Query);
    let root_type_name = root_type_name_for_operation_kind(supergraph_state, operation_kind);
    let operation = &fetch.operation.document.operation;
    let default_list_size = default_list_size_for_subgraph(config, &fetch.service_name);
    let cost = estimate_operation_cost(
        operation,
        &fetch.operation.document.fragments,
        root_type_name,
        operation_kind,
        supergraph_state,
        variable_payload,
        default_list_size,
        fragments,
    );

    out.estimated_cost = out.estimated_cost.saturating_add(cost);
    let subgraph_cost = out.per_subgraph.entry(&fetch.service_name).or_insert(0);
    *subgraph_cost = subgraph_cost.saturating_add(cost);
}

fn root_type_name_for_operation_kind<'a>(
    supergraph_state: &'a SupergraphState,
    operation_kind: &OperationKind,
) -> &'a str {
    match operation_kind {
        OperationKind::Query => supergraph_state.query_type.as_str(),
        OperationKind::Mutation => supergraph_state
            .mutation_type
            .as_deref()
            .unwrap_or(supergraph_state.query_type.as_str()),
        OperationKind::Subscription => supergraph_state
            .subscription_type
            .as_deref()
            .unwrap_or(supergraph_state.query_type.as_str()),
    }
}

fn estimate_operation_cost<'exec>(
    operation: &'exec OperationDefinition,
    operation_fragments: &'exec [FragmentDefinition],
    root_type_name: &'exec str,
    operation_kind: &'exec OperationKind,
    supergraph_state: &'exec SupergraphState,
    variable_payload: &'exec CoerceVariablesPayload,
    default_list_size: usize,
    fragments_cache: &mut HashMap<&'exec str, &'exec FragmentDefinition>,
) -> u64 {
    for fragment in operation_fragments {
        fragments_cache.insert(fragment.name.as_str(), fragment);
    }

    let operation_base: u64 = match operation_kind {
        OperationKind::Mutation => 10,
        OperationKind::Query | OperationKind::Subscription => 0,
    };

    operation_base.saturating_add(estimate_selection_set_cost(
        &operation.selection_set,
        root_type_name,
        supergraph_state,
        variable_payload,
        default_list_size,
        fragments_cache,
        &mut HashSet::new(),
        &[],
    ))
}

fn estimate_selection_set_cost<'exec>(
    selection_set: &'exec SelectionSet,
    parent_type_name: &'exec str,
    supergraph_state: &'exec SupergraphState,
    variable_payload: &'exec CoerceVariablesPayload,
    default_list_size: usize,
    fragments_cache: &'exec HashMap<&'exec str, &'exec FragmentDefinition>,
    visited_fragments: &mut HashSet<&'exec str>,
    inherited_sized_paths: &[(Vec<String>, usize)],
) -> u64 {
    let mut total_cost = 0_u64;

    for item in &selection_set.items {
        match item {
            SelectionItem::Field(field) => {
                total_cost = total_cost.saturating_add(estimate_field_selection_cost(
                    field,
                    parent_type_name,
                    supergraph_state,
                    variable_payload,
                    default_list_size,
                    fragments_cache,
                    visited_fragments,
                    inherited_sized_paths,
                ));
            }
            SelectionItem::InlineFragment(fragment) => {
                total_cost = total_cost.saturating_add(estimate_selection_set_cost(
                    &fragment.selections,
                    fragment.type_condition.as_str(),
                    supergraph_state,
                    variable_payload,
                    default_list_size,
                    fragments_cache,
                    visited_fragments,
                    inherited_sized_paths,
                ));
            }
            SelectionItem::FragmentSpread(fragment_name) => {
                if visited_fragments.contains(fragment_name.as_str()) {
                    continue;
                }

                let Some(fragment_def) = fragments_cache.get(fragment_name.as_str()) else {
                    continue;
                };

                visited_fragments.insert(fragment_name);
                total_cost = total_cost.saturating_add(estimate_selection_set_cost(
                    &fragment_def.selection_set,
                    fragment_def.type_condition.as_str(),
                    supergraph_state,
                    variable_payload,
                    default_list_size,
                    fragments_cache,
                    visited_fragments,
                    inherited_sized_paths,
                ));
                visited_fragments.remove(fragment_name.as_str());
            }
        }
    }

    total_cost
}

fn estimate_field_selection_cost<'exec>(
    field: &'exec FieldSelection,
    parent_type_name: &'exec str,
    supergraph_state: &'exec SupergraphState,
    variable_payload: &'exec CoerceVariablesPayload,
    default_list_size: usize,
    fragments_cache: &'exec HashMap<&'exec str, &'exec FragmentDefinition>,
    visited_fragments: &mut HashSet<&'exec str>,
    inherited_sized_paths: &[(Vec<String>, usize)],
) -> u64 {
    if !is_conditionally_included(field, variable_payload) {
        return 0;
    }

    let field_def = supergraph_state
        .definitions
        .get(parent_type_name)
        .and_then(|definition| definition.fields().get(field.name.as_str()));

    let (return_type_name, field_type, field_base_cost) = if let Some(definition) = field_def {
        let mut base = definition
            .cost
            .as_ref()
            .map(|cost| cost.weight)
            .unwrap_or(0);

        if let Some(arguments) = &field.arguments {
            for key in arguments.keys() {
                if let Some(cost) = definition.cost_by_arguments.get(key) {
                    base = base.saturating_add(cost.weight);
                }

                if let Some(argument_type) = definition.argument_types.get(key) {
                    if let Some(argument_value) = arguments.get_argument(key) {
                        base = base.saturating_add(estimate_input_value_cost(
                            argument_value,
                            argument_type,
                            supergraph_state,
                            variable_payload,
                        ));
                    }
                }
            }
        }

        (
            definition.field_type.inner_type(),
            &definition.field_type,
            base,
        )
    } else {
        (
            parent_type_name,
            &TypeNode::Named(parent_type_name.to_string()),
            0,
        )
    };

    let return_type_cost = type_cost(supergraph_state, return_type_name);

    let mut inherited_for_children = Vec::<(Vec<String>, usize)>::new();
    let mut inherited_size_for_current = None;
    for (path, size) in inherited_sized_paths {
        if path.first().is_none_or(|segment| segment != &field.name) {
            continue;
        }

        if path.len() == 1 {
            inherited_size_for_current = Some(inherited_size_for_current.unwrap_or(0).max(*size));
        } else {
            inherited_for_children.push((path[1..].to_vec(), *size));
        }
    }

    let mut explicit_list_size = None;

    if let Some(definition) = field_def {
        if let Some(list_size_directive) = &definition.list_size {
            let configured_size = evaluate_list_size_directive(
                list_size_directive,
                field,
                variable_payload,
                default_list_size,
            );

            if let Some(sized_fields) = &list_size_directive.sized_fields {
                for path in sized_fields {
                    let parsed_path = parse_sized_field_path(path);
                    if !parsed_path.is_empty() {
                        inherited_for_children.push((parsed_path, configured_size));
                    }
                }
            } else if field_type.is_list() {
                explicit_list_size = Some(configured_size);
            }
        }
    }

    if explicit_list_size.is_none() && field_type.is_list() {
        explicit_list_size = Some(inherited_size_for_current.unwrap_or(default_list_size));
    }

    let child_cost = estimate_selection_set_cost(
        &field.selections,
        return_type_name,
        supergraph_state,
        variable_payload,
        default_list_size,
        fragments_cache,
        visited_fragments,
        &inherited_for_children,
    );

    if let Some(list_size) = explicit_list_size {
        let per_item = return_type_cost.saturating_add(child_cost);
        field_base_cost.saturating_add((list_size as u64).saturating_mul(per_item))
    } else {
        field_base_cost
            .saturating_add(return_type_cost)
            .saturating_add(child_cost)
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

fn evaluate_list_size_directive(
    directive: &hive_router_query_planner::federation_spec::demand_control::ListSizeDirective,
    field: &FieldSelection,
    variable_payload: &CoerceVariablesPayload,
    default_list_size: usize,
) -> usize {
    if let Some(assumed_size) = directive.assumed_size {
        return assumed_size;
    }

    let Some(slicing_arguments) = &directive.slicing_arguments else {
        return default_list_size;
    };

    let Some(arguments) = field.arguments.as_ref() else {
        return default_list_size;
    };

    let mut values = vec![];

    for slicing_arg in slicing_arguments {
        let segments = slicing_arg.split('.').collect::<Vec<_>>();
        if segments.is_empty() {
            continue;
        }

        let Some(root_arg) = arguments.get_argument(segments[0]) else {
            continue;
        };

        let nested = if segments.len() == 1 {
            resolve_integer_value(root_arg, &[], variable_payload)
        } else {
            resolve_integer_value(root_arg, &segments[1..], variable_payload)
        };

        if let Some(v) = nested {
            values.push(v as usize);
        }
    }

    if directive.require_one_slicing_argument {
        if values.len() == 1 {
            values[0]
        } else {
            default_list_size
        }
    } else {
        values.into_iter().max().unwrap_or(default_list_size)
    }
}

fn resolve_integer_value(
    value: &AstValue,
    path: &[&str],
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
            let Some((head, tail)) = path.split_first() else {
                return None;
            };

            let nested_value = object.get(*head)?;
            resolve_integer_value(nested_value, tail, variable_payload)
        }
        _ => None,
    }
}

fn resolve_integer_from_json_value(value: &Value, path: &[&str]) -> Option<u64> {
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

fn parse_sized_field_path(path: &str) -> Vec<String> {
    let mut current = String::new();
    let mut out = Vec::new();

    for ch in path.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            current.push(ch);
        } else if !current.is_empty() {
            out.push(current.clone());
            current.clear();
        }
    }

    if !current.is_empty() {
        out.push(current);
    }

    out
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

fn is_conditionally_included(
    field: &FieldSelection,
    variable_payload: &CoerceVariablesPayload,
) -> bool {
    if let Some(skip_if) = &field.skip_if {
        if variable_payload.variable_equals_true(skip_if) {
            return false;
        }
    }

    if let Some(include_if) = &field.include_if {
        return variable_payload.variable_equals_true(include_if);
    }

    true
}
