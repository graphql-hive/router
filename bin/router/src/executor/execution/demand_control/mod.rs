pub mod subgraph_response_tracker;

use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

use crate::config::demand_control::{
    DemandControlActualCostMode, DemandControlExposeHeadersConfig, DemandControlMode,
};
use crate::query_planner::{
    ast::{
        document::Document,
        operation::OperationDefinition,
        selection_item::SelectionItem,
        selection_set::{FieldSelection, SelectionSet},
        value::Value as AstValue,
    },
    state::supergraph_state::{SupergraphDefinition, SupergraphState, TypeNode},
};
use crate::telemetry::{
    metrics::demand_control_metrics::{DemandControlMetricsRecorder, DemandControlResultCode},
    traces::spans::graphql::GraphQLOperationSpan,
};
use ahash::{HashMap as AHashMap, HashSet as AHashSet};
use http::HeaderValue;
use sonic_rs::JsonValueTrait;

use crate::executor::{
    execution::{
        demand_control::subgraph_response_tracker::SubgraphResponseCostTracker,
        plan::{ExecutionJob, VariablesMap},
    },
    headers::{plan::HeaderAggregationStrategy, response::ResponseHeaderAggregator},
    response::value::Value,
};

#[derive(Debug)]
pub struct DemandControlEvaluation {
    /// Total estimation of the cost of the operation, across all subgraphs.
    pub estimated_cost: u64,
    /// Estimated cost per subgraph.
    pub per_subgraph: Arc<BTreeMap<String, u64>>,
}

impl DemandControlEvaluation {
    pub fn estimated_cost_for_subgraph(&self, subgraph_name: &str) -> u64 {
        self.per_subgraph
            .get(subgraph_name)
            .copied()
            .unwrap_or_default()
    }
}

#[derive(Debug)]
pub struct DemandControlExecutionOperationContext {
    pub operation_max_cost: u64,
    pub expose_headers_flags: Arc<DemandControlExposeHeadersConfig>,
}

#[derive(Debug)]
pub struct DemandControlExecutionSubgraphsContext {
    pub enforcement_mode: DemandControlMode,
    /// Subgraphs whose estimated cost exceeded their per-subgraph max. The
    /// map value is the configured per-subgraph max (used when synthesising
    /// the rejection error).
    pub blocked_subgraphs: BTreeMap<String, u64>,
    /// How to handle subgraphs whose estimated cost exceeds their per-subgraph max.
    /// This is handed to the subgraph execution layer to determine whether
    /// execution is blocked or not.
    pub blocked_subgraphs_enforcement_mode: DemandControlMode,
}

impl DemandControlExecutionSubgraphsContext {
    pub fn is_subgraph_blocked(&self, subgraph_name: &str) -> bool {
        self.blocked_subgraphs.contains_key(subgraph_name)
    }
}

#[derive(Debug)]
pub struct DemandControlExecutionActualCostContext {
    pub cost_mode: DemandControlActualCostMode,
    pub cost_plan: Arc<CompiledActualCostPlan>,
}

#[derive(Debug)]
pub struct DemandControlExecutionContext {
    pub evaluation: DemandControlEvaluation,
    pub metrics_recorder: Option<DemandControlMetricsRecorder>,
    pub operation: DemandControlExecutionOperationContext,
    pub subgraphs: DemandControlExecutionSubgraphsContext,
    pub actual: DemandControlExecutionActualCostContext,
}

impl DemandControlExecutionContext {
    #[inline]
    pub fn report_telemetry(
        &self,
        actual: u64,
        operation_name: Option<&str>,
        operation_span: &GraphQLOperationSpan,
    ) {
        let delta = actual as i128 - self.evaluation.estimated_cost as i128;
        let delta_i64 = delta.max(i64::MIN as i128).min(i64::MAX as i128) as i64;
        let result_code = DemandControlResultCode::from_artifacts(
            self.operation.operation_max_cost,
            self.evaluation.estimated_cost,
            actual,
        );

        operation_span.record_demand_control(
            self.evaluation.estimated_cost,
            Some(actual),
            Some(delta_i64),
            &result_code,
        );

        if let Some(metrics_recorder) = &self.metrics_recorder {
            metrics_recorder.record_actual_cost(actual, &result_code, operation_name);
            metrics_recorder.record_delta(delta as f64, &result_code, operation_name);
        }
    }

    #[inline]
    pub fn calculate_actual_cost(
        &self,
        data: &Value<'_>,
        variable_values: &Option<HashMap<String, sonic_rs::Value>>,
        subgraph_response_cost_tracker: &SubgraphResponseCostTracker,
    ) -> u64 {
        match self.actual.cost_plan.as_ref() {
            CompiledActualCostPlan::BySubgraph(_) => subgraph_response_cost_tracker.total(),
            CompiledActualCostPlan::ByResponseShape(actual_response_shape_plan) => {
                estimate_actual_response_shape_cost_with_compiled_plan(
                    actual_response_shape_plan,
                    data,
                    variable_values,
                )
            }
        }
    }

    #[inline]
    pub fn record_subgraph_response_cost<'exec>(
        &self,
        subgraph_response_tracker: &mut SubgraphResponseCostTracker<'exec>,
        job: &ExecutionJob<'exec>,
        variables_map: &'exec Option<VariablesMap>,
    ) {
        if let CompiledActualCostPlan::BySubgraph(actual_subgraph_plans_by_fetch_hash) =
            self.actual.cost_plan.as_ref()
        {
            let fetch_step_hash = job.operation().hash;
            let response_ref = job.response_ref();

            let fetch_step_cost_actual = actual_subgraph_plans_by_fetch_hash
                .get(&fetch_step_hash)
                .map(|plan| {
                    estimate_actual_subgraph_response_cost_with_compiled_plan(
                        plan,
                        &response_ref.data,
                        variables_map,
                    )
                })
                .unwrap_or(0);

            subgraph_response_tracker.track(
                job.subgraph_name(),
                fetch_step_hash,
                fetch_step_cost_actual,
            );
        }
    }

    #[inline]
    pub fn apply_expose_headers(
        &self,
        response_header_aggregator: &mut ResponseHeaderAggregator,
        actual_cost: u64,
    ) {
        if let Some(header_name) = &self.operation.expose_headers_flags.actual {
            response_header_aggregator.write(
                header_name.get_header_ref(),
                &HeaderValue::from(actual_cost),
                HeaderAggregationStrategy::Last,
            );
        }

        if let Some(header_name) = &self.operation.expose_headers_flags.estimated {
            response_header_aggregator.write(
                header_name.get_header_ref(),
                &HeaderValue::from(self.evaluation.estimated_cost),
                HeaderAggregationStrategy::Last,
            );
        }

        if let Some(header_name) = &self.operation.expose_headers_flags.max {
            response_header_aggregator.write(
                header_name.get_header_ref(),
                &HeaderValue::from(self.operation.operation_max_cost),
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
    /// The object types the fragment applies to. `None` when it's on its parent's own type, like
    /// `products { ... on Product @include(if: $v) { ... } }`, so it applies to every object,
    /// with or without `__typename`.
    applies_to: Option<AHashSet<String>>,
    child: CompiledSelectionSetActualCostPlan,
}

pub fn compile_actual_subgraph_cost_plan(
    document: &Document,
    supergraph_state: &SupergraphState,
) -> CompiledSubgraphActualCostPlan {
    let operation_def = &document.operation;
    let root_type_name =
        supergraph_state.expect_root_type_name(operation_def.operation_kind.as_ref());

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
                        applies_to: (fragment.type_condition != parent_type_name).then(|| {
                            // `... on Product` applies to a `Book`, and to a `Product` too, which
                            // is what a subgraph with `Product` as an `@interfaceObject` returns
                            let mut types: AHashSet<String> = supergraph_state
                                .all_possible_types(&fragment.type_condition)
                                .into_iter()
                                .flatten()
                                .cloned()
                                .collect();
                            types.insert(fragment.type_condition.clone());
                            types
                        }),
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
                if !inline_fragment_applies(parent_value, fragment.applies_to.as_ref()) {
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

fn inline_fragment_applies(
    parent_value: &Value<'_>,
    applies_to: Option<&AHashSet<String>>,
) -> bool {
    let Some(applies_to) = applies_to else {
        return true;
    };
    parent_value
        .as_object()
        .and_then(|obj| response_object_get(obj, "__typename"))
        .and_then(|value| value.as_str())
        .is_some_and(|typename| applies_to.contains(typename))
}

#[inline]
fn response_object_get<'a>(obj: &'a [(&'a str, Value<'a>)], key: &str) -> Option<&'a Value<'a>> {
    obj.iter().find(|(k, _)| *k == key).map(|(_, v)| v)
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use graphql_tools::parser::query::Definition;

    use super::{
        compile_actual_response_shape_cost_plan, compile_actual_subgraph_cost_plan,
        estimate_actual_response_shape_cost_with_compiled_plan,
        estimate_actual_subgraph_response_cost_with_compiled_plan,
    };
    use crate::executor::response::value::Value;
    use crate::query_planner::{
        ast::{document::Document, normalization::normalize_operation},
        planner::plan_nodes::planning::PlanNode,
        state::supergraph_state::SupergraphState,
        tests::testkit::{build_query_plan_with_defaults, read_supergraph},
        utils::parsing::{parse_operation, parse_schema},
    };

    const FIXTURE: &str = "fixture/tests/abstract-types.supergraph.graphql";

    /// The actual cost of `query` for one `Book` and one `Magazine`, with the client's
    /// operation and with the subgraph's
    fn actual_costs(query: &str) -> (u64, u64) {
        let schema = parse_schema(&read_supergraph(FIXTURE));
        let supergraph = SupergraphState::new(&schema);
        let variables = Some(HashMap::from([("v".to_string(), sonic_rs::json!(true))]));
        let json: sonic_rs::Value = sonic_rs::from_str(
            r#"{"products":[
              {"__typename":"Book","dimensions":{"size":"1"}},
              {"__typename":"Magazine","dimensions":{"size":"2"}}
            ]}"#,
        )
        .unwrap();
        let data = Value::from(json.as_ref());

        let normalized = normalize_operation(&supergraph, &parse_operation(query), None).unwrap();
        let plan = compile_actual_response_shape_cost_plan(
            normalized.executable_operation(),
            "Query",
            &supergraph,
        );
        let by_response_shape =
            estimate_actual_response_shape_cost_with_compiled_plan(&plan, &data, &variables);

        let query_plan = build_query_plan_with_defaults(FIXTURE, parse_operation(query)).unwrap();
        let Some(PlanNode::Fetch(fetch)) = &query_plan.node else {
            panic!("expected a single fetch, got {query_plan}");
        };
        let plan = compile_actual_subgraph_cost_plan(&fetch.operation.document, &supergraph);
        let by_subgraph =
            estimate_actual_subgraph_response_cost_with_compiled_plan(&plan, &data, &variables);

        (by_response_shape, by_subgraph)
    }

    /// `... on Product @include(if: $v)` stays on `Product`, the interface, and a `Book` is still
    /// a `Product`, so its fields count
    #[test]
    fn conditional_fragment_on_an_interface_counts_its_fields() {
        let plain = actual_costs("{ products { dimensions { size } } }");
        assert_eq!(plain, (4, 4));
        assert_eq!(
            actual_costs(
                "query($v: Boolean!) { products { ... on Product @include(if: $v) { dimensions { size } } } }"
            ),
            plain
        );
    }

    /// A subgraph that has an interface as an `@interfaceObject` returns the interface's own name
    /// as `__typename`. `... on Similar` applies to a `"Similar"` like it does to a `"Book"`.
    /// Planning doesn't make a query like this today, so it's written by hand
    #[test]
    fn fragment_applies_to_its_own_type_name() {
        let schema = parse_schema(&read_supergraph(FIXTURE));
        let supergraph = SupergraphState::new(&schema);
        let query = parse_operation("{ products { ... on Similar { similar { id } } } }");
        let Some(Definition::Operation(operation)) = query.definitions.into_iter().next() else {
            panic!("expected an operation");
        };
        let document = Document {
            operation: operation.into(),
            fragments: vec![],
        };
        let plan = compile_actual_subgraph_cost_plan(&document, &supergraph);
        let cost = |typename: &str| {
            let json: sonic_rs::Value = sonic_rs::from_str(&format!(
                r#"{{"products":[{{"__typename":"{typename}","similar":[{{"id":"1"}}]}}]}}"#
            ))
            .unwrap();
            estimate_actual_subgraph_response_cost_with_compiled_plan(
                &plan,
                &Value::from(json.as_ref()),
                &None,
            )
        };

        assert_eq!(cost("Similar"), cost("Book"));
        // and the fragment's fields do count, a `User` isn't `Similar`
        assert!(cost("Book") > cost("User"));
    }
}
