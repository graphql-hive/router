use std::collections::{HashMap, HashSet};

use hive_router_config::demand_control::DemandControlActualCostMode;
use hive_router_internal::telemetry::metrics::demand_control_metrics::{
    DemandControlMetricsRecorder, DemandControlResultCode,
};
use hive_router_query_planner::{
    ast::{
        operation::{OperationDefinition, SubgraphFetchOperation},
        selection_item::SelectionItem,
        selection_set::{FieldSelection, SelectionSet},
    },
    state::supergraph_state::{OperationKind, SupergraphDefinition, SupergraphState, TypeNode},
};
use serde::Serialize;
use sonic_rs::JsonValueTrait;

use crate::response::value::Value;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DemandControlResponseExtensions<'exec> {
    pub estimated: u64,
    pub result: &'exec DemandControlResultCode,
    pub by_subgraph: &'exec HashMap<&'exec str, u64>,
    pub blocked_subgraphs: &'exec HashSet<&'exec str>,

    pub max_cost: Option<u64>,

    pub actual: Option<u64>,
    pub delta: Option<i64>,
    pub actual_by_subgraph: Option<&'exec HashMap<&'exec str, u64>>,
}

#[derive(Debug, Clone, Default)]
pub struct DemandControlEvaluation<'exec> {
    pub estimated_cost: u64,
    pub per_subgraph: HashMap<&'exec str, u64>,
}

#[derive(Debug)]
pub struct DemandControlExecutionContext<'exec> {
    pub max_cost: Option<u64>,
    pub evaluation: DemandControlEvaluation<'exec>,
    pub blocked_subgraphs: HashSet<&'exec str>,
    pub operation_name: Option<&'exec str>,
    pub actual_cost_mode: Option<DemandControlActualCostMode>,
    pub result_code: DemandControlResultCode,
    pub metrics_recorder: Option<DemandControlMetricsRecorder>,
    pub include_extension_metadata: bool,
}

pub fn demand_control_actual_cost<'exec>(
    demand_control: &DemandControlExecutionContext<'exec>,
    supergraph_state: &'exec SupergraphState,
    operation: &OperationDefinition,
    root_type_name: &str,
    data: &Value<'_>,
    variable_values: &Option<HashMap<String, sonic_rs::Value>>,
    actual_cost_by_subgraph_from_responses: &'exec HashMap<&'exec str, u64>,
) -> Option<DemandControlActualCostResult<'exec>> {
    let mut actual_by_subgraph = None;
    let actual = match demand_control.actual_cost_mode? {
        DemandControlActualCostMode::BySubgraph => {
            let total = actual_cost_by_subgraph_from_responses.values().sum();
            actual_by_subgraph = Some(actual_cost_by_subgraph_from_responses);
            total
        }
        DemandControlActualCostMode::ByResponseShape => {
            estimate_actual_operation_cost_from_response_shape(
                operation,
                root_type_name,
                data,
                supergraph_state,
                variable_values,
            )
        }
    };

    let delta = actual as i128 - demand_control.evaluation.estimated_cost as i128;
    let delta_i64 = delta.max(i64::MIN as i128).min(i64::MAX as i128) as i64;

    let max_cost_exceeded = demand_control.max_cost.filter(|max| actual > *max);

    let result_code = if max_cost_exceeded.is_some() {
        DemandControlResultCode::CostActualTooExpensive
    } else if demand_control.blocked_subgraphs.is_empty() {
        DemandControlResultCode::CostOk
    } else {
        DemandControlResultCode::CostEstimatedTooExpensive
    };

    if let Some(metrics_recorder) = demand_control.metrics_recorder.as_ref() {
        metrics_recorder.record_actual_cost(actual, &result_code, demand_control.operation_name);
        metrics_recorder.record_delta(delta_i64, &result_code, demand_control.operation_name);
    }

    Some(DemandControlActualCostResult {
        actual,
        max_cost_exceeded,
        actual_by_subgraph,
        result_code,
        delta: delta_i64,
    })
}

pub struct DemandControlActualCostResult<'exec> {
    pub actual: u64,
    pub delta: i64,
    pub max_cost_exceeded: Option<u64>,
    pub actual_by_subgraph: Option<&'exec HashMap<&'exec str, u64>>,
    pub result_code: DemandControlResultCode,
}

pub fn estimate_actual_subgraph_response_cost_from_response_shape(
    operation: &SubgraphFetchOperation,
    response_data: &Value<'_>,
    supergraph_state: &SupergraphState,
    variable_values: &Option<HashMap<String, sonic_rs::Value>>,
) -> u64 {
    let operation_def = &operation.document.operation;
    let root_type_name = match operation_def
        .operation_kind
        .as_ref()
        .unwrap_or(&OperationKind::Query)
    {
        OperationKind::Query => supergraph_state.query_type.as_str(),
        OperationKind::Mutation => supergraph_state
            .mutation_type
            .as_deref()
            .unwrap_or("Mutation"),
        OperationKind::Subscription => supergraph_state
            .subscription_type
            .as_deref()
            .unwrap_or("Subscription"),
    };

    if operation.document.operation.selection_set.items.len() == 1 {
        if let SelectionItem::Field(field) = &operation.document.operation.selection_set.items[0] {
            if field.name == "_entities" && field.alias.is_none() {
                let entities = response_data
                    .as_object()
                    .and_then(|obj| response_object_get(obj, "_entities"))
                    .and_then(|value| match value {
                        Value::Array(items) => Some(items),
                        _ => None,
                    });

                let Some(entities) = entities else {
                    return 0;
                };

                let mut total = 0_u64;
                for entity in entities {
                    let entity_type = entity
                        .as_object()
                        .and_then(|obj| response_object_get(obj, "__typename"))
                        .and_then(|value| value.as_str())
                        .unwrap_or("_Entity");

                    total = total.saturating_add(
                        estimate_actual_selection_set_cost_from_response_shape(
                            operation.get_inner_selection_set(),
                            entity_type,
                            entity,
                            supergraph_state,
                            variable_values,
                        ),
                    );
                }

                return total;
            }
        }
    }

    estimate_actual_selection_set_cost_from_response_shape(
        &operation_def.selection_set,
        root_type_name,
        response_data,
        supergraph_state,
        variable_values,
    )
}

fn estimate_actual_operation_cost_from_response_shape(
    operation: &OperationDefinition,
    root_type_name: &str,
    data: &Value<'_>,
    supergraph_state: &SupergraphState,
    variable_values: &Option<HashMap<String, sonic_rs::Value>>,
) -> u64 {
    estimate_actual_selection_set_cost_from_response_shape(
        &operation.selection_set,
        root_type_name,
        data,
        supergraph_state,
        variable_values,
    )
}

fn estimate_actual_selection_set_cost_from_response_shape(
    selection_set: &SelectionSet,
    parent_type_name: &str,
    parent_value: &Value<'_>,
    supergraph_state: &SupergraphState,
    variable_values: &Option<HashMap<String, sonic_rs::Value>>,
) -> u64 {
    let mut total_cost = 0_u64;

    for item in &selection_set.items {
        match item {
            SelectionItem::Field(field) => {
                total_cost = total_cost.saturating_add(
                    estimate_actual_field_selection_cost_from_response_shape(
                        field,
                        parent_type_name,
                        parent_value,
                        supergraph_state,
                        variable_values,
                    ),
                );
            }
            SelectionItem::InlineFragment(fragment) => {
                if should_skip_inline_fragment(parent_value, &fragment.type_condition) {
                    continue;
                }

                total_cost = total_cost.saturating_add(
                    estimate_actual_selection_set_cost_from_response_shape(
                        &fragment.selections,
                        fragment.type_condition.as_str(),
                        parent_value,
                        supergraph_state,
                        variable_values,
                    ),
                );
            }
            SelectionItem::FragmentSpread(_) => {
                // Normalized operations used for planning are expected to inline fragment spreads.
            }
        }
    }

    total_cost
}

fn estimate_actual_field_selection_cost_from_response_shape(
    field: &FieldSelection,
    parent_type_name: &str,
    parent_value: &Value<'_>,
    supergraph_state: &SupergraphState,
    variable_values: &Option<HashMap<String, sonic_rs::Value>>,
) -> u64 {
    if !is_conditionally_included_for_actual(field, variable_values) {
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

    let response_key = field.selection_identifier();
    let value = parent_value
        .as_object()
        .and_then(|obj| response_object_get(obj, response_key));

    let return_type_cost = dc_type_cost(supergraph_state, return_type_name);

    if field_type.is_list() {
        let Some(items) = value.and_then(|v| match v {
            Value::Array(items) => Some(items),
            _ => None,
        }) else {
            return field_base_cost;
        };

        let mut list_total = 0_u64;
        for item in items.iter() {
            let child = estimate_actual_selection_set_cost_from_response_shape(
                &field.selections,
                return_type_name,
                item,
                supergraph_state,
                variable_values,
            );
            list_total = list_total.saturating_add(return_type_cost.saturating_add(child));
        }

        return field_base_cost.saturating_add(list_total);
    }

    let Some(value) = value else {
        return field_base_cost;
    };

    if value.is_null() {
        return field_base_cost;
    }

    let child = estimate_actual_selection_set_cost_from_response_shape(
        &field.selections,
        return_type_name,
        value,
        supergraph_state,
        variable_values,
    );

    field_base_cost
        .saturating_add(return_type_cost)
        .saturating_add(child)
}

fn should_skip_inline_fragment(parent_value: &Value<'_>, type_condition: &str) -> bool {
    let typename = parent_value
        .as_object()
        .and_then(|obj| response_object_get(obj, "__typename"))
        .and_then(|value| value.as_str());

    if let Some(typename) = typename {
        return typename != type_condition;
    }

    false
}

fn response_object_get<'a>(obj: &'a [(&'a str, Value<'a>)], key: &str) -> Option<&'a Value<'a>> {
    obj.binary_search_by_key(&key, |(k, _)| *k)
        .ok()
        .map(|idx| &obj[idx].1)
}

fn dc_type_cost(supergraph_state: &SupergraphState, type_name: &str) -> u64 {
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

fn is_conditionally_included_for_actual(
    field: &FieldSelection,
    variable_values: &Option<HashMap<String, sonic_rs::Value>>,
) -> bool {
    if let Some(skip_if) = &field.skip_if {
        if variable_equals_true(variable_values, skip_if) {
            return false;
        }
    }

    if let Some(include_if) = &field.include_if {
        return variable_equals_true(variable_values, include_if);
    }

    true
}

fn variable_equals_true(
    variable_values: &Option<HashMap<String, sonic_rs::Value>>,
    variable_name: &str,
) -> bool {
    variable_values
        .as_ref()
        .and_then(|vars| vars.get(variable_name))
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}
