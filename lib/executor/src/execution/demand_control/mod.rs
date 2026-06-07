pub mod extensions;

use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

use ahash::{HashMap as AHashMap, HashSet as AHashSet};
use hive_router_config::demand_control::{
    DemandControlActualCostMode, DemandControlExposeHeadersConfig, DemandControlMode,
};
use hive_router_internal::telemetry::metrics::demand_control_metrics::DemandControlMetricsRecorder;
use hive_router_query_planner::{
    ast::{
        operation::{OperationDefinition, SubgraphFetchOperation},
        selection_item::SelectionItem,
        selection_set::{FieldSelection, SelectionSet},
        value::Value as AstValue,
    },
    state::supergraph_state::{SupergraphDefinition, SupergraphState, TypeNode},
};
use http::HeaderValue;
use sonic_rs::JsonValueTrait;

use crate::{
    headers::{plan::HeaderAggregationStrategy, response::ResponseHeaderAggregator},
    response::value::Value,
};

#[derive(Debug)]
pub struct DemandControlEvaluation {
    pub estimated_cost: u64,
    pub per_subgraph: Arc<BTreeMap<String, u64>>,
}

#[derive(Debug)]
pub struct DemandControlExecutionContext {
    pub mode: DemandControlMode,
    pub max_cost: u64,
    pub evaluation: DemandControlEvaluation,
    /// Subgraphs whose estimated cost exceeded their per-subgraph max. The
    /// map value is the configured per-subgraph max (used when synthesising
    /// the rejection error). Whether execution is actually blocked for these
    /// subgraphs is determined by `mode`: in `Enforce` mode the call is
    /// short-circuited with `CostEstimatedTooExpensive`, in `Measure` mode
    /// the request still runs but the per-subgraph result code is recorded
    /// in the response extension.
    pub subgraphs_over_limit: BTreeMap<String, u64>,
    pub metrics_recorder: Option<DemandControlMetricsRecorder>,
    pub expose_headers_flags: Arc<DemandControlExposeHeadersConfig>,
    pub actual_cost_mode: DemandControlActualCostMode,
    pub actual_cost_plan: Arc<CompiledActualCostPlan>,
}

impl DemandControlExecutionContext {
    #[inline]
    pub fn apply_expose_headers(
        &self,
        response_header_aggregator: &mut ResponseHeaderAggregator,
        actual_cost: u64,
    ) {
        if let Some(header_name) = &self.expose_headers_flags.actual {
            response_header_aggregator.write(
                header_name.get_header_ref(),
                &HeaderValue::from(actual_cost),
                HeaderAggregationStrategy::Last,
            );
        }

        if let Some(header_name) = &self.expose_headers_flags.estimated {
            response_header_aggregator.write(
                header_name.get_header_ref(),
                &HeaderValue::from(self.evaluation.estimated_cost),
                HeaderAggregationStrategy::Last,
            );
        }

        if let Some(header_name) = &self.expose_headers_flags.max {
            response_header_aggregator.write(
                header_name.get_header_ref(),
                &HeaderValue::from(self.max_cost),
                HeaderAggregationStrategy::Last,
            );
        }
    }
}

#[derive(Debug)]
pub enum CompiledActualCostPlan {
    BySubgraph(AHashMap<u64, CompiledSubgraphActualCostPlan>),
    ByResponseShape(CompiledResponseShapeActualCostPlan),
}

#[derive(Debug)]
pub struct CompiledSubgraphActualCostPlan {
    root: CompiledActualCostRootPlan,
}

#[derive(Debug)]
pub struct CompiledResponseShapeActualCostPlan {
    root: CompiledSelectionSetActualCostPlan,
}

#[derive(Debug)]
enum CompiledActualCostRootPlan {
    SelectionSet(CompiledSelectionSetActualCostPlan),
    /// One or more `_entities` groups, keyed by response key (field name or alias).
    /// Handles both FlattenFetch (`_entities`) and BatchFetch (`_e0: _entities`, `_e1: _entities`).
    EntityGroups(Vec<CompiledEntityGroup>),
}

#[derive(Debug)]
struct CompiledEntityGroup {
    response_key: String,
    entity_plans_by_type: AHashMap<String, CompiledEntityTypePlan>,
}

#[derive(Debug)]
struct CompiledEntityTypePlan {
    /// Cost of the entity's own type (e.g. 1 for an Object, or whatever
    /// the type's `@cost` weight is). Charged once per entity returned by
    /// `_entities`, on top of the child selection cost.
    type_cost: u64,
    selections: CompiledSelectionSetActualCostPlan,
}

#[derive(Debug, Default)]
struct CompiledSelectionSetActualCostPlan {
    items: Vec<CompiledSelectionItemActualCostPlan>,
}

#[derive(Debug)]
enum CompiledSelectionItemActualCostPlan {
    Field(CompiledFieldActualCostPlan),
    InlineFragment(CompiledInlineFragmentActualCostPlan),
}

#[derive(Debug)]
struct CompiledFieldActualCostPlan {
    response_key: String,
    field_base_cost: u64,
    return_type_cost: u64,
    is_list: bool,
    include_if: Option<String>,
    skip_if: Option<String>,
    child: CompiledSelectionSetActualCostPlan,
}

#[derive(Debug)]
struct CompiledInlineFragmentActualCostPlan {
    type_condition: String,
    // If parent and fragment type are the same at compile time, the fragment
    // deterministically applies even when runtime __typename is not present.
    apply_when_typename_missing: bool,
    child: CompiledSelectionSetActualCostPlan,
}

pub fn compile_actual_subgraph_cost_plan(
    operation: &SubgraphFetchOperation,
    supergraph_state: &SupergraphState,
) -> CompiledSubgraphActualCostPlan {
    let operation_def = &operation.document.operation;
    let root_type_name = supergraph_state.root_type_name(operation_def.operation_kind.as_ref());

    // Detect if every top-level selection is a `_entities` field (with or without alias).
    // This covers FlattenFetch (single `_entities`) and BatchFetch (multiple `_eN: _entities`).
    let all_entity_fields = !operation_def.selection_set.items.is_empty()
        && operation_def
            .selection_set
            .items
            .iter()
            .all(|item| matches!(item, SelectionItem::Field(f) if f.name == "_entities"));

    if all_entity_fields {
        let mut groups = Vec::with_capacity(operation_def.selection_set.items.len());

        for item in &operation_def.selection_set.items {
            let SelectionItem::Field(field) = item else {
                continue;
            };

            let response_key = field
                .alias
                .as_deref()
                .unwrap_or(field.name.as_str())
                .to_string();

            let mut referenced_entity_types = AHashSet::default();
            collect_entity_root_type_conditions(&field.selections, &mut referenced_entity_types);

            let mut entity_plans_by_type = AHashMap::default();
            for type_name in &referenced_entity_types {
                entity_plans_by_type.insert(
                    type_name.clone(),
                    CompiledEntityTypePlan {
                        type_cost: demand_control_definition_cost(supergraph_state, type_name),
                        selections: compile_selection_set_actual_cost_plan(
                            &field.selections,
                            type_name,
                            supergraph_state,
                        ),
                    },
                );
            }

            groups.push(CompiledEntityGroup {
                response_key,
                entity_plans_by_type,
            });
        }

        return CompiledSubgraphActualCostPlan {
            root: CompiledActualCostRootPlan::EntityGroups(groups),
        };
    }

    CompiledSubgraphActualCostPlan {
        root: CompiledActualCostRootPlan::SelectionSet(compile_selection_set_actual_cost_plan(
            &operation_def.selection_set,
            root_type_name,
            supergraph_state,
        )),
    }
}

pub fn compile_actual_response_shape_cost_plan(
    operation: &OperationDefinition,
    root_type_name: &str,
    supergraph_state: &SupergraphState,
) -> CompiledResponseShapeActualCostPlan {
    CompiledResponseShapeActualCostPlan {
        root: compile_selection_set_actual_cost_plan(
            &operation.selection_set,
            root_type_name,
            supergraph_state,
        ),
    }
}

fn collect_entity_root_type_conditions(selection_set: &SelectionSet, out: &mut AHashSet<String>) {
    for item in &selection_set.items {
        if let SelectionItem::InlineFragment(fragment) = item {
            out.insert(fragment.type_condition.clone());
        }
    }
}

pub fn estimate_actual_subgraph_response_cost_with_compiled_plan(
    plan: &CompiledSubgraphActualCostPlan,
    response_data: &Value<'_>,
    variable_values: &Option<HashMap<String, sonic_rs::Value>>,
) -> u64 {
    match &plan.root {
        CompiledActualCostRootPlan::SelectionSet(selection_set) => {
            evaluate_selection_set_actual_cost_plan(selection_set, response_data, variable_values)
        }
        CompiledActualCostRootPlan::EntityGroups(groups) => {
            let mut total = 0_u64;
            for group in groups {
                let entities = response_data
                    .as_object()
                    .and_then(|obj| response_object_get(obj, group.response_key.as_str()))
                    .and_then(|value| match value {
                        Value::Array(items) => Some(items),
                        _ => None,
                    });

                let Some(entities) = entities else {
                    continue;
                };

                for entity in entities.iter() {
                    let entity_type = entity
                        .as_object()
                        .and_then(|obj| response_object_get(obj, "__typename"))
                        .and_then(|value| value.as_str());

                    // If typename is present, look it up in the map. Otherwise, when the map
                    // has exactly one entry (the common federation case where each _entities fetch
                    // targets a single type), use that plan — all representations in this fetch
                    // are of that type.
                    let entity_plan = entity_type
                        .and_then(|t| group.entity_plans_by_type.get(t))
                        .or_else(|| {
                            if group.entity_plans_by_type.len() == 1 {
                                group.entity_plans_by_type.values().next()
                            } else {
                                None
                            }
                        });

                    let Some(entity_plan) = entity_plan else {
                        continue;
                    };

                    // Charge the entity's own type cost once per returned
                    // entity (mirrors the per-item cost of a list field),
                    // then add the cost of walking its selections.
                    total = total.saturating_add(entity_plan.type_cost);
                    total = total.saturating_add(evaluate_selection_set_actual_cost_plan(
                        &entity_plan.selections,
                        entity,
                        variable_values,
                    ));
                }
            }

            total
        }
    }
}

pub fn estimate_actual_response_shape_cost_with_compiled_plan(
    plan: &CompiledResponseShapeActualCostPlan,
    response_data: &Value<'_>,
    variable_values: &Option<HashMap<String, sonic_rs::Value>>,
) -> u64 {
    evaluate_selection_set_actual_cost_plan(&plan.root, response_data, variable_values)
}

fn compile_selection_set_actual_cost_plan(
    selection_set: &SelectionSet,
    parent_type_name: &str,
    supergraph_state: &SupergraphState,
) -> CompiledSelectionSetActualCostPlan {
    let mut items = Vec::with_capacity(selection_set.items.len());

    for item in &selection_set.items {
        match item {
            SelectionItem::Field(field) => items.push(CompiledSelectionItemActualCostPlan::Field(
                compile_field_actual_cost_plan(field, parent_type_name, supergraph_state),
            )),
            SelectionItem::InlineFragment(fragment) => {
                items.push(CompiledSelectionItemActualCostPlan::InlineFragment(
                    CompiledInlineFragmentActualCostPlan {
                        type_condition: fragment.type_condition.clone(),
                        apply_when_typename_missing: fragment.type_condition == parent_type_name,
                        child: compile_selection_set_actual_cost_plan(
                            &fragment.selections,
                            fragment.type_condition.as_str(),
                            supergraph_state,
                        ),
                    },
                ))
            }
            SelectionItem::FragmentSpread(_) => {
                // Normalized operations used for planning are expected to inline fragment spreads.
            }
        }
    }

    CompiledSelectionSetActualCostPlan { items }
}

fn compile_field_actual_cost_plan(
    field: &FieldSelection,
    parent_type_name: &str,
    supergraph_state: &SupergraphState,
) -> CompiledFieldActualCostPlan {
    // `__typename` is a built-in introspection field returning String. It
    // never appears in the parent type's `fields()` map, so the generic
    // fallback below would compute `return_type_cost = dc_type_cost(parent)`
    // (e.g. 1 for an interface/union/object), which would wrongly inflate
    // the actual cost. Treat it as a free scalar.
    if field.name.as_str() == "__typename" {
        return CompiledFieldActualCostPlan {
            response_key: field.selection_identifier().to_string(),
            field_base_cost: 0,
            return_type_cost: 0,
            is_list: false,
            include_if: field.include_if.clone(),
            skip_if: field.skip_if.clone(),
            child: CompiledSelectionSetActualCostPlan { items: Vec::new() },
        };
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
            for (key, value) in arguments {
                if let Some(cost) = definition.cost_by_arguments.get(key) {
                    base = base.saturating_add(cost.weight);
                }
                // Recursively account for `@cost` on input field definitions
                // referenced through this argument's value (e.g. an input
                // object whose fields carry `@cost(weight: ...)`). These
                // contribute to the actual cost as well, mirroring the
                // estimated cost behaviour. Variables are not resolved at
                // compile time and contribute 0 here (a TODO for
                // variable-driven inputs).
                if let Some(arg_type) = definition.argument_types.get(key) {
                    base = base.saturating_add(compile_input_value_cost(
                        value,
                        arg_type,
                        supergraph_state,
                    ));
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

    CompiledFieldActualCostPlan {
        response_key: field.selection_identifier().to_string(),
        field_base_cost,
        return_type_cost: demand_control_definition_cost(supergraph_state, return_type_name),
        is_list: field_type.is_list(),
        include_if: field.include_if.clone(),
        skip_if: field.skip_if.clone(),
        child: compile_selection_set_actual_cost_plan(
            &field.selections,
            return_type_name,
            supergraph_state,
        ),
    }
}

fn evaluate_selection_set_actual_cost_plan(
    plan: &CompiledSelectionSetActualCostPlan,
    parent_value: &Value<'_>,
    variable_values: &Option<HashMap<String, sonic_rs::Value>>,
) -> u64 {
    let mut total_cost = 0_u64;

    for item in &plan.items {
        match item {
            CompiledSelectionItemActualCostPlan::Field(field) => {
                total_cost = total_cost.saturating_add(evaluate_field_actual_cost_plan(
                    field,
                    parent_value,
                    variable_values,
                ));
            }
            CompiledSelectionItemActualCostPlan::InlineFragment(fragment) => {
                if should_skip_inline_fragment(
                    parent_value,
                    &fragment.type_condition,
                    fragment.apply_when_typename_missing,
                ) {
                    continue;
                }

                total_cost = total_cost.saturating_add(evaluate_selection_set_actual_cost_plan(
                    &fragment.child,
                    parent_value,
                    variable_values,
                ));
            }
        }
    }

    total_cost
}

fn evaluate_field_actual_cost_plan(
    field: &CompiledFieldActualCostPlan,
    parent_value: &Value<'_>,
    variable_values: &Option<HashMap<String, sonic_rs::Value>>,
) -> u64 {
    if !is_conditionally_included_for_actual_from_flags(
        field.include_if.as_deref(),
        field.skip_if.as_deref(),
        variable_values,
    ) {
        return 0;
    }

    let value = parent_value
        .as_object()
        .and_then(|obj| response_object_get(obj, field.response_key.as_str()));

    if field.is_list {
        let Some(items) = value.and_then(|v| match v {
            Value::Array(items) => Some(items),
            _ => None,
        }) else {
            return field.field_base_cost;
        };

        let mut list_total = 0_u64;
        for item in items.iter() {
            let child =
                evaluate_selection_set_actual_cost_plan(&field.child, item, variable_values);
            list_total = list_total.saturating_add(field.return_type_cost.saturating_add(child));
        }

        return field.field_base_cost.saturating_add(list_total);
    }

    let Some(value) = value else {
        return field.field_base_cost;
    };

    if value.is_null() {
        return field.field_base_cost;
    }

    let child = evaluate_selection_set_actual_cost_plan(&field.child, value, variable_values);

    field
        .field_base_cost
        .saturating_add(field.return_type_cost)
        .saturating_add(child)
}

#[inline]
pub fn calculate_actual_cost(
    demand_control: &DemandControlExecutionContext,
    data: &Value<'_>,
    variable_values: &Option<HashMap<String, sonic_rs::Value>>,
    actual_cost_by_subgraph_from_responses: &Option<ahash::HashMap<&str, u64>>,
) -> u64 {
    match demand_control.actual_cost_plan.as_ref() {
        CompiledActualCostPlan::BySubgraph(_) => {
            if let Some(actual_cost_by_subgraph_from_responses) =
                actual_cost_by_subgraph_from_responses
            {
                actual_cost_by_subgraph_from_responses
                    .values()
                    .fold(0u64, |acc, cost| acc.saturating_add(*cost))
            } else {
                0
            }
        }
        CompiledActualCostPlan::ByResponseShape(actual_response_shape_plan) => {
            estimate_actual_response_shape_cost_with_compiled_plan(
                actual_response_shape_plan,
                data,
                variable_values,
            )
        }
    }
}

fn should_skip_inline_fragment(
    parent_value: &Value<'_>,
    type_condition: &str,
    apply_when_typename_missing: bool,
) -> bool {
    let typename = parent_value
        .as_object()
        .and_then(|obj| response_object_get(obj, "__typename"))
        .and_then(|value| value.as_str());

    if let Some(typename) = typename {
        return typename != type_condition;
    }

    !apply_when_typename_missing
}

fn response_object_get<'a>(obj: &'a [(&'a str, Value<'a>)], key: &str) -> Option<&'a Value<'a>> {
    obj.binary_search_by_key(&key, |(k, _)| *k)
        .ok()
        .map(|idx| &obj[idx].1)
}

pub fn demand_control_definition_cost(supergraph_state: &SupergraphState, type_name: &str) -> u64 {
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

/// Computes the additional cost contribution of a literal argument value
/// (used for `field_base_cost` precomputation in the actual cost plan).
///
/// Rules:
/// - For each input-object instance, charges a default cost of `1` plus the
///   per-field `@cost(weight)` declared on each present input field, plus
///   the recursive cost of the field's value.
/// - For lists, sums the cost of each element.
/// - For scalar/enum values and `null`, contributes `0`.
/// - For `Variable` references, contributes `0` here (variables would need
///   runtime resolution; literals cover all parity fixtures).
fn compile_input_value_cost(
    value: &AstValue,
    value_type: &TypeNode,
    supergraph_state: &SupergraphState,
) -> u64 {
    match value {
        AstValue::Object(map) => {
            let TypeNode::Named(type_name) = value_type.unwrap_non_null() else {
                return 0;
            };
            let Some(SupergraphDefinition::InputObject(input_object)) =
                supergraph_state.definitions.get(type_name)
            else {
                return 0;
            };

            let mut total: u64 = 1; // default per input-object instance
            for (field_name, field_value) in map {
                let Some(input_field) = input_object.fields.get(field_name) else {
                    continue;
                };
                let field_cost = input_field
                    .cost
                    .as_ref()
                    .map(|cost| cost.weight)
                    .unwrap_or(0);
                total = total
                    .saturating_add(field_cost)
                    .saturating_add(compile_input_value_cost(
                        field_value,
                        &input_field.field_type,
                        supergraph_state,
                    ));
            }
            total
        }
        AstValue::List(items) => {
            let TypeNode::List(inner_type) = value_type.unwrap_non_null() else {
                return 0;
            };
            items
                .iter()
                .map(|item| compile_input_value_cost(item, inner_type, supergraph_state))
                .fold(0u64, |acc, c| acc.saturating_add(c))
        }
        AstValue::Null
        | AstValue::Variable(_)
        | AstValue::Int(_)
        | AstValue::Float(_)
        | AstValue::String(_)
        | AstValue::Boolean(_)
        | AstValue::Enum(_) => 0,
    }
}

fn is_conditionally_included_for_actual_from_flags(
    include_if: Option<&str>,
    skip_if: Option<&str>,
    variable_values: &Option<HashMap<String, sonic_rs::Value>>,
) -> bool {
    if let Some(skip_if) = skip_if {
        if variable_equals_true(variable_values, skip_if) {
            return false;
        }
    }

    if let Some(include_if) = include_if {
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
