use ahash::AHashMap;
use bytes::BufMut;
use hive_router_query_planner::planner::plan_nodes::{
    CompiledSelectionItem, CompiledSelectionSet, DeferNode, DeferredNode, FetchNode, PlanNode,
    QueryPlan, RuntimeCompiledFieldSelection, RuntimeCompiledInlineFragmentSelection,
    RuntimeCompiledSelectionItem, RuntimeCompiledSelectionSet,
};
use hive_router_query_planner::planner::plan_nodes::{SchemaInterner, TypeId};
use std::sync::Arc;

use crate::{
    introspection::schema::PossibleTypes,
    json_writer::{write_and_escape_string, write_f64, write_i64, write_u64},
    response::flat::{FlatResponseData, FlatValue, ValueId},
    utils::consts::{
        CLOSE_BRACE, CLOSE_BRACKET, COLON, COMMA, FALSE, OPEN_BRACE, OPEN_BRACKET, QUOTE, TRUE,
        TYPENAME, TYPENAME_FIELD_NAME,
    },
};

fn write_response_key(first: bool, response_key: Option<&str>, buffer: &mut Vec<u8>) {
    if !first {
        buffer.put(COMMA);
    }
    if let Some(response_key) = response_key {
        buffer.put(QUOTE);
        buffer.put(response_key.as_bytes());
        buffer.put(QUOTE);
        buffer.put(COLON);
    }
}

#[derive(Debug, Clone, Default)]
pub struct StaticRequiresSelectionSet {
    pub items: Vec<StaticRequiresSelectionItem>,
}

#[derive(Debug, Clone)]
pub struct StaticRequiresFieldSelection {
    pub field_name: String,
    pub selections: StaticRequiresSelectionSet,
}

#[derive(Debug, Clone)]
pub struct StaticRequiresInlineFragmentSelection {
    pub type_condition: TypeId,
    pub selections: StaticRequiresSelectionSet,
}

#[derive(Debug, Clone)]
pub enum StaticRequiresSelectionItem {
    Field(StaticRequiresFieldSelection),
    InlineFragment(StaticRequiresInlineFragmentSelection),
}

pub type StaticRequiresRegistry = AHashMap<i64, Arc<StaticRequiresSelectionSet>>;

pub fn compile_static_requires_registry(
    query_plan: &QueryPlan,
    interner: &SchemaInterner,
) -> StaticRequiresRegistry {
    let mut registry = StaticRequiresRegistry::new();

    if let Some(node) = &query_plan.node {
        collect_static_requires_from_node(node, interner, &mut registry);
    }

    registry
}

pub fn bind_runtime_selection_set(
    data: &FlatResponseData,
    selection_set: &StaticRequiresSelectionSet,
) -> RuntimeCompiledSelectionSet {
    let items = selection_set
        .items
        .iter()
        .map(|item| match item {
            StaticRequiresSelectionItem::Field(field) => {
                RuntimeCompiledSelectionItem::Field(RuntimeCompiledFieldSelection {
                    symbol: data.symbol_for(&field.field_name),
                    selections: bind_runtime_selection_set(data, &field.selections),
                })
            }
            StaticRequiresSelectionItem::InlineFragment(fragment) => {
                RuntimeCompiledSelectionItem::InlineFragment(
                    RuntimeCompiledInlineFragmentSelection {
                        type_condition: fragment.type_condition,
                        selections: bind_runtime_selection_set(data, &fragment.selections),
                    },
                )
            }
        })
        .collect();

    RuntimeCompiledSelectionSet { items }
}

fn collect_static_requires_from_node(
    node: &PlanNode,
    interner: &SchemaInterner,
    registry: &mut StaticRequiresRegistry,
) {
    match node {
        PlanNode::Fetch(fetch) => maybe_insert_fetch_requires(fetch, interner, registry),
        PlanNode::Flatten(flatten) => {
            collect_static_requires_from_node(&flatten.node, interner, registry)
        }
        PlanNode::Parallel(parallel) => {
            for child in &parallel.nodes {
                collect_static_requires_from_node(child, interner, registry);
            }
        }
        PlanNode::Sequence(sequence) => {
            for child in &sequence.nodes {
                collect_static_requires_from_node(child, interner, registry);
            }
        }
        PlanNode::Condition(condition) => {
            if let Some(node) = &condition.if_clause {
                collect_static_requires_from_node(node, interner, registry);
            }
            if let Some(node) = &condition.else_clause {
                collect_static_requires_from_node(node, interner, registry);
            }
        }
        PlanNode::Subscription(subscription) => {
            collect_static_requires_from_node(&subscription.primary, interner, registry);
        }
        PlanNode::Defer(defer) => collect_static_requires_from_defer(defer, interner, registry),
    }
}

fn collect_static_requires_from_defer(
    defer: &DeferNode,
    interner: &SchemaInterner,
    registry: &mut StaticRequiresRegistry,
) {
    if let Some(node) = &defer.primary.node {
        collect_static_requires_from_node(node, interner, registry);
    }
    for deferred in &defer.deferred {
        collect_static_requires_from_deferred_node(deferred, interner, registry);
    }
}

fn collect_static_requires_from_deferred_node(
    deferred: &DeferredNode,
    interner: &SchemaInterner,
    registry: &mut StaticRequiresRegistry,
) {
    if let Some(node) = &deferred.node {
        collect_static_requires_from_node(node, interner, registry);
    }
}

fn maybe_insert_fetch_requires(
    fetch: &FetchNode,
    interner: &SchemaInterner,
    registry: &mut StaticRequiresRegistry,
) {
    let Some(compiled_requires) = fetch.compiled_requires.as_ref() else {
        return;
    };
    registry.insert(
        fetch.id,
        Arc::new(compile_static_requires_selection_set(
            compiled_requires,
            interner,
        )),
    );
}

fn compile_static_requires_selection_set(
    selection_set: &CompiledSelectionSet,
    interner: &SchemaInterner,
) -> StaticRequiresSelectionSet {
    let items = selection_set
        .items
        .iter()
        .map(|item| match item {
            CompiledSelectionItem::Field(field) => {
                StaticRequiresSelectionItem::Field(StaticRequiresFieldSelection {
                    field_name: interner.resolve_field(&field.name).to_string(),
                    selections: compile_static_requires_selection_set(&field.selections, interner),
                })
            }
            CompiledSelectionItem::InlineFragment(fragment) => {
                StaticRequiresSelectionItem::InlineFragment(StaticRequiresInlineFragmentSelection {
                    type_condition: fragment.type_condition,
                    selections: compile_static_requires_selection_set(
                        &fragment.selections,
                        interner,
                    ),
                })
            }
        })
        .collect();

    StaticRequiresSelectionSet { items }
}

pub fn project_requires_flat<'i>(
    interner: &'i SchemaInterner,
    type_name_cache: &mut AHashMap<TypeId, &'i str>,
    data: &FlatResponseData,
    possible_types: &PossibleTypes,
    requires_selections: &RuntimeCompiledSelectionSet,
    entity_id: ValueId,
    buffer: &mut Vec<u8>,
    first: bool,
    response_key: Option<&str>,
) -> bool {
    match data.value_kind(entity_id) {
        Some(FlatValue::Null) | None => return false,
        Some(FlatValue::Bool(b)) => {
            write_response_key(first, response_key, buffer);
            buffer.put(if *b { TRUE } else { FALSE });
        }
        Some(FlatValue::F64(n)) => {
            write_response_key(first, response_key, buffer);
            write_f64(buffer, *n);
        }
        Some(FlatValue::I64(n)) => {
            write_response_key(first, response_key, buffer);
            write_i64(buffer, *n);
        }
        Some(FlatValue::U64(n)) => {
            write_response_key(first, response_key, buffer);
            write_u64(buffer, *n);
        }
        Some(FlatValue::String(s)) => {
            write_response_key(first, response_key, buffer);
            write_and_escape_string(buffer, s.as_ref());
        }
        Some(FlatValue::List(list_id)) => {
            write_response_key(first, response_key, buffer);
            buffer.put(OPEN_BRACKET);
            let mut first = true;
            for item_id in data.list_items(*list_id).unwrap_or(&[]) {
                let projected = project_requires_flat(
                    interner,
                    type_name_cache,
                    data,
                    possible_types,
                    requires_selections,
                    *item_id,
                    buffer,
                    first,
                    None,
                );
                if projected {
                    first = false;
                }
            }
            buffer.put(CLOSE_BRACKET);
        }
        Some(FlatValue::Object(object_id)) => {
            if requires_selections.items.is_empty() {
                write_response_key(first, response_key, buffer);
                serialize_flat_value_to_buffer(data, entity_id, buffer);
                return true;
            }

            let entity_obj = data.object_fields(*object_id).unwrap_or(&[]);
            if entity_obj.is_empty() {
                return false;
            }

            let parent_first = first;
            let mut first = true;
            project_requires_map_mut(
                interner,
                type_name_cache,
                data,
                possible_types,
                requires_selections,
                entity_id,
                buffer,
                &mut first,
                response_key,
                parent_first,
            );
            if first {
                return false;
            }
            buffer.put(CLOSE_BRACE);
        }
    };

    true
}

fn project_requires_map_mut<'i>(
    interner: &'i SchemaInterner,
    type_name_cache: &mut AHashMap<TypeId, &'i str>,
    data: &FlatResponseData,
    possible_types: &PossibleTypes,
    requires_selections: &RuntimeCompiledSelectionSet,
    entity_id: ValueId,
    buffer: &mut Vec<u8>,
    first: &mut bool,
    parent_response_key: Option<&str>,
    parent_first: bool,
) {
    for requires_selection in &requires_selections.items {
        match requires_selection {
            RuntimeCompiledSelectionItem::Field(requires_selection) => {
                let Some(symbol) = requires_selection.symbol else {
                    continue;
                };
                let response_key = data.field_name(symbol).unwrap_or_default();
                if response_key == TYPENAME_FIELD_NAME {
                    continue;
                }

                let original = data.value_kind(entity_id).and_then(|value| match value {
                    FlatValue::Object(object_id) => {
                        data.object_field_in_object_by_symbol(*object_id, symbol)
                    }
                    _ => None,
                });

                let Some(original) = original else {
                    continue;
                };

                if matches!(data.value_kind(original), Some(FlatValue::Null) | None) {
                    continue;
                }

                if *first {
                    write_response_key(parent_first, parent_response_key, buffer);
                    buffer.put(OPEN_BRACE);

                    if let Some(type_name) = data
                        .object_field(entity_id, TYPENAME_FIELD_NAME)
                        .and_then(|id| data.value_as_str(id))
                    {
                        buffer.put(QUOTE);
                        buffer.put(TYPENAME);
                        buffer.put(QUOTE);
                        buffer.put(COLON);
                        write_and_escape_string(buffer, type_name);
                        *first = false;
                    }
                }

                let projected = project_requires_flat(
                    interner,
                    type_name_cache,
                    data,
                    possible_types,
                    &requires_selection.selections,
                    original,
                    buffer,
                    *first,
                    Some(response_key),
                );
                if projected {
                    *first = false;
                }
            }
            RuntimeCompiledSelectionItem::InlineFragment(requires_selection) => {
                let type_condition = *type_name_cache
                    .entry(requires_selection.type_condition)
                    .or_insert_with(|| interner.resolve_type(&requires_selection.type_condition));
                let type_name = data
                    .object_field(entity_id, TYPENAME_FIELD_NAME)
                    .and_then(|id| data.value_as_str(id))
                    .unwrap_or(type_condition);

                if possible_types.entity_satisfies_type_condition(type_name, type_condition)
                    || possible_types.entity_satisfies_type_condition(type_condition, type_name)
                {
                    project_requires_map_mut(
                        interner,
                        type_name_cache,
                        data,
                        possible_types,
                        &requires_selection.selections,
                        entity_id,
                        buffer,
                        first,
                        parent_response_key,
                        parent_first,
                    );
                }
            }
        }
    }
}

fn serialize_flat_value_to_buffer(data: &FlatResponseData, id: ValueId, buffer: &mut Vec<u8>) {
    match data.value_kind(id) {
        Some(FlatValue::Null) | None => buffer.put(b"null".as_slice()),
        Some(FlatValue::Bool(true)) => buffer.put(TRUE),
        Some(FlatValue::Bool(false)) => buffer.put(FALSE),
        Some(FlatValue::U64(num)) => write_u64(buffer, *num),
        Some(FlatValue::I64(num)) => write_i64(buffer, *num),
        Some(FlatValue::F64(num)) => write_f64(buffer, *num),
        Some(FlatValue::String(value)) => write_and_escape_string(buffer, value),
        Some(FlatValue::Object(obj_id)) => {
            buffer.put(OPEN_BRACE);
            let mut first = true;
            for (key, value_id) in data.object_fields(*obj_id).unwrap_or(&[]) {
                if !first {
                    buffer.put(COMMA);
                }
                write_and_escape_string(buffer, key.as_ref());
                buffer.put(COLON);
                serialize_flat_value_to_buffer(data, *value_id, buffer);
                first = false;
            }
            buffer.put(CLOSE_BRACE);
        }
        Some(FlatValue::List(list_id)) => {
            buffer.put(OPEN_BRACKET);
            let mut first = true;
            for child in data.list_items(*list_id).unwrap_or(&[]) {
                if !first {
                    buffer.put(COMMA);
                }
                serialize_flat_value_to_buffer(data, *child, buffer);
                first = false;
            }
            buffer.put(CLOSE_BRACKET);
        }
    }
}
