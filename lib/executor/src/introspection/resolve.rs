use bumpalo::Bump;
use std::sync::Arc;

use graphql_tools::parser::query::Value as QueryValue;
use graphql_tools::static_graphql::schema::{
    Definition, Directive, DirectiveDefinition, Document, EnumValue, Field, InputValue, Type,
    TypeDefinition,
};

use hive_router_query_planner::ast::{
    operation::OperationDefinition,
    selection_item::SelectionItem,
    selection_set::{FieldSelection, SelectionSet},
    value::Value as AstValue,
};
use sonic_rs::JsonValueTrait;

use crate::execution::plan::CoerceVariablesPayload;
use crate::introspection::schema::SchemaMetadata;
use crate::response::value::Value;
use hive_router_query_planner::planner::merged_shape::response_shape_for_selections;

pub struct IntrospectionContext {
    pub query: Option<Arc<OperationDefinition>>,
    pub schema: Arc<Document>,
    pub metadata: Arc<SchemaMetadata>,
    pub variables: Arc<CoerceVariablesPayload>,
}

/// Places named entries into the slots the response tree uses at this position.
///
/// The tree carries values by slot, so introspection — which resolves fields by name —
/// converts once here, against the same shape rule the query planner applies to the client
/// operation. Introspection is a cold path, so building the shape per object is fine.
fn into_slots<'a>(
    entries: Vec<(&str, Value<'a>)>,
    selections: &SelectionSet,
    arena: &'a Bump,
) -> Value<'a> {
    let shape = response_shape_for_selections(selections);
    let slots: &mut [Value<'a>] = arena.alloc_slice_fill_default(shape.fields.len());
    for (key, value) in entries {
        if let Some(slot) = shape.slot_of(key) {
            slots[slot] = value;
        }
    }
    Value::Object(slots)
}

/// Copies a schema string into the arena.
///
/// The response tree outlives the borrow of the schema it is read from, so introspection
/// values own nothing they did not put in the arena. Introspection is a cold path, and these
/// are short names.
#[inline]
fn str_value<'a>(arena: &'a Bump, text: &str) -> Value<'a> {
    Value::String(arena.alloc_str(text))
}

fn resolve_boolean_variable(
    var_name: &str,
    variables: &Arc<CoerceVariablesPayload>,
) -> Option<bool> {
    variables
        .variables_map
        .as_ref()
        .and_then(|map| map.get(var_name))
        .and_then(|value| value.as_bool())
}

fn resolve_str_variable<'a>(
    var_name: &str,
    variables: &'a Arc<CoerceVariablesPayload>,
) -> Option<&'a str> {
    variables
        .variables_map
        .as_ref()
        .and_then(|map| map.get(var_name))
        .and_then(|value| value.as_str())
}

fn get_deprecation_reason(directives: &[Directive]) -> Option<&str> {
    directives
        .iter()
        .find(|d| d.name == "deprecated")
        .and_then(|d| {
            d.arguments
                .iter()
                .find(|(name, _)| name.as_str() == "reason")
        })
        .and_then(|(_, value)| {
            if let QueryValue::String(s) = value {
                Some(s.as_str())
            } else {
                None
            }
        })
}

fn is_deprecated(directives: &[Directive]) -> bool {
    directives.iter().any(|d| d.name == "deprecated")
}

fn is_deprecated_enum(enum_val: &EnumValue) -> bool {
    is_deprecated(&enum_val.directives)
}

fn get_specified_by_url(directives: &[Directive]) -> Option<&str> {
    directives
        .iter()
        .find(|d| d.name == "specifiedBy")
        .and_then(|d| d.arguments.iter().find(|(name, _)| name.as_str() == "url"))
        .and_then(|(_, value)| {
            if let QueryValue::String(s) = value {
                Some(s.as_str())
            } else {
                None
            }
        })
}

fn is_one_of(directives: &[Directive]) -> bool {
    directives.iter().any(|d| d.name == "oneOf")
}

fn kind_to_str(type_def: &TypeDefinition) -> &'static str {
    match type_def {
        TypeDefinition::Scalar(_) => "SCALAR",
        TypeDefinition::Object(_) => "OBJECT",
        TypeDefinition::Interface(_) => "INTERFACE",
        TypeDefinition::Union(_) => "UNION",
        TypeDefinition::Enum(_) => "ENUM",
        TypeDefinition::InputObject(_) => "INPUT_OBJECT",
    }
}

fn resolve_input_value<'exec, 'a>(
    iv: &'exec InputValue,
    selections: &'exec SelectionSet,
    ctx: &'exec IntrospectionContext,
    arena: &'a Bump,
) -> Value<'a> {
    into_slots(
        resolve_input_value_selections(iv, &selections.items, ctx, arena),
        selections,
        arena,
    )
}

fn resolve_input_value_selections<'exec, 'a>(
    iv: &'exec InputValue,
    selection_items: &'exec Vec<SelectionItem>,
    ctx: &'exec IntrospectionContext,
    arena: &'a Bump,
) -> Vec<(&'exec str, Value<'a>)> {
    let mut iv_data: Vec<(&str, Value<'_>)> = Vec::with_capacity(selection_items.len());
    for item in selection_items {
        if let SelectionItem::Field(field) = item {
            let value = match field.name.as_str() {
                "name" => str_value(arena, iv.name.as_str()),
                "description" => iv
                    .description
                    .as_ref()
                    .map_or(Value::Null, |s| str_value(arena, s.as_str())),
                "type" => resolve_type(&iv.value_type, &field.selections, ctx, arena),
                "defaultValue" => iv
                    .default_value
                    .as_ref()
                    .map_or_else(|| Value::Null, |ast| str_value(arena, &ast.to_string())), // TODO: support default values
                "isDeprecated" => Value::Bool(is_deprecated(&iv.directives)),
                "deprecationReason" => get_deprecation_reason(&iv.directives)
                    .map_or(Value::Null, |s| str_value(arena, s)),
                "__typename" => Value::String("__InputValue"),
                _ => Value::Null,
            };
            iv_data.push((field.selection_identifier(), value));
        } else if let SelectionItem::InlineFragment(_) = item {
            let selection_items = item.selections();
            if let Some(selection_items) = selection_items {
                let new_data = resolve_input_value_selections(iv, selection_items, ctx, arena);
                iv_data.extend(new_data);
            }
        }
    }
    iv_data
}

fn resolve_field<'exec, 'a>(
    f: &'exec Field,
    selections: &'exec SelectionSet,
    ctx: &'exec IntrospectionContext,
    arena: &'a Bump,
) -> Value<'a> {
    into_slots(
        resolve_field_selections(f, &selections.items, ctx, arena),
        selections,
        arena,
    )
}

fn resolve_field_selections<'exec, 'a>(
    f: &'exec Field,
    selection_items: &'exec Vec<SelectionItem>,
    ctx: &'exec IntrospectionContext,
    arena: &'a Bump,
) -> Vec<(&'exec str, Value<'a>)> {
    let mut field_data = Vec::with_capacity(selection_items.len());
    for item in selection_items {
        if let SelectionItem::Field(field) = item {
            let value = match field.name.as_str() {
                "name" => str_value(arena, f.name.as_str()),
                "description" => f
                    .description
                    .as_ref()
                    .map_or(Value::Null, |s| str_value(arena, s.as_str())),
                "args" => {
                    let args: Vec<_> = f
                        .arguments
                        .iter()
                        .map(|arg| resolve_input_value(arg, &field.selections, ctx, arena))
                        .collect();
                    Value::Array(arena.alloc_slice_fill_iter(args))
                }
                "type" => resolve_type(&f.field_type, &field.selections, ctx, arena),
                "isDeprecated" => Value::Bool(is_deprecated(&f.directives)),
                "deprecationReason" => get_deprecation_reason(&f.directives)
                    .map_or(Value::Null, |s| str_value(arena, s)),
                "__typename" => Value::String("__Field"),
                _ => Value::Null,
            };
            field_data.push((field.selection_identifier(), value));
        } else if let SelectionItem::InlineFragment(_) = item {
            let selection_items = item.selections();
            if let Some(selection_items) = selection_items {
                let new_data = resolve_field_selections(f, selection_items, ctx, arena);
                field_data.extend(new_data);
            }
        }
    }
    field_data
}

fn resolve_enum_value<'exec, 'a>(
    ev: &'exec EnumValue,
    selections: &'exec SelectionSet,
    arena: &'a Bump,
) -> Value<'a> {
    into_slots(
        resolve_enum_value_selections(ev, &selections.items, arena),
        selections,
        arena,
    )
}

fn resolve_enum_value_selections<'exec, 'a>(
    ev: &'exec EnumValue,
    selection_items: &'exec Vec<SelectionItem>,
    arena: &'a Bump,
) -> Vec<(&'exec str, Value<'a>)> {
    let mut ev_data = Vec::with_capacity(selection_items.len());
    for item in selection_items {
        if let SelectionItem::Field(field) = item {
            let value = match field.name.as_str() {
                "name" => str_value(arena, ev.name.as_str()),
                "description" => ev
                    .description
                    .as_ref()
                    .map_or(Value::Null, |s| str_value(arena, s.as_str())),
                "isDeprecated" => Value::Bool(is_deprecated_enum(ev)),
                "deprecationReason" => get_deprecation_reason(&ev.directives)
                    .map_or(Value::Null, |s| str_value(arena, s)),
                "__typename" => Value::String("__EnumValue"),
                _ => Value::Null,
            };
            ev_data.push((field.selection_identifier(), value));
        } else if let SelectionItem::InlineFragment(_) = item {
            let selection_items = item.selections();
            if let Some(selection_items) = selection_items {
                let new_data = resolve_enum_value_selections(ev, selection_items, arena);
                ev_data.extend(new_data);
            }
        }
    }
    ev_data
}

fn resolve_type_definition<'exec, 'a>(
    type_def: &'exec TypeDefinition,
    selections: &'exec SelectionSet,
    ctx: &'exec IntrospectionContext,
    arena: &'a Bump,
) -> Value<'a> {
    into_slots(
        resolve_type_definition_selections(type_def, &selections.items, ctx, arena),
        selections,
        arena,
    )
}

fn resolve_type_definition_selections<'exec, 'a>(
    type_def: &'exec TypeDefinition,
    selection_items: &'exec Vec<SelectionItem>,
    ctx: &'exec IntrospectionContext,
    arena: &'a Bump,
) -> Vec<(&'exec str, Value<'a>)> {
    let mut type_data = Vec::with_capacity(selection_items.len());

    for item in selection_items {
        if let SelectionItem::Field(field) = item {
            let value = match field.name.as_str() {
                "kind" => Value::String(kind_to_str(type_def)),
                "name" => match type_def {
                    TypeDefinition::Scalar(s) => Some(&s.name),
                    TypeDefinition::Object(o) => Some(&o.name),
                    TypeDefinition::Interface(i) => Some(&i.name),
                    TypeDefinition::Union(u) => Some(&u.name),
                    TypeDefinition::Enum(e) => Some(&e.name),
                    TypeDefinition::InputObject(io) => Some(&io.name),
                }
                .map(|s| str_value(arena, s.as_str()))
                .unwrap_or(Value::Null),
                "description" => match type_def {
                    TypeDefinition::Scalar(s) => s.description.as_ref(),
                    TypeDefinition::Object(o) => o.description.as_ref(),
                    TypeDefinition::Interface(i) => i.description.as_ref(),
                    TypeDefinition::Union(u) => u.description.as_ref(),
                    TypeDefinition::Enum(e) => e.description.as_ref(),
                    TypeDefinition::InputObject(io) => io.description.as_ref(),
                }
                .map_or(Value::Null, |s| str_value(arena, s.as_str())),
                "specifiedByURL" => {
                    if let TypeDefinition::Scalar(scalar) = type_def {
                        get_specified_by_url(&scalar.directives)
                            .map_or(Value::Null, |s| str_value(arena, s))
                    } else {
                        Value::Null
                    }
                }
                "isOneOf" => {
                    if let TypeDefinition::InputObject(type_def) = type_def {
                        Value::Bool(is_one_of(&type_def.directives))
                    } else {
                        Value::Null
                    }
                }
                "fields" => {
                    let fields = match type_def {
                        TypeDefinition::Object(o) => Some(&o.fields),
                        TypeDefinition::Interface(i) => Some(&i.fields),
                        _ => None,
                    };
                    if let Some(fields) = fields {
                        let include_deprecated = field
                            .arguments
                            .as_ref()
                            .and_then(|a| a.get_argument("includeDeprecated"))
                            .and_then(|v| match v {
                                AstValue::Boolean(b) => Some(*b),
                                AstValue::Variable(var_name) => {
                                    resolve_boolean_variable(var_name.as_str(), &ctx.variables)
                                }
                                _ => None,
                            })
                            .unwrap_or(false);

                        let fields_values: Vec<Value<'a>> = fields
                            .iter()
                            .filter(|f| {
                                !f.name.starts_with("__")
                                    && (include_deprecated || !is_deprecated(&f.directives))
                            })
                            .map(|f| resolve_field(f, &field.selections, ctx, arena))
                            .collect();
                        Value::Array(arena.alloc_slice_fill_iter(fields_values))
                    } else {
                        Value::Null
                    }
                }
                "interfaces" => {
                    if let TypeDefinition::Object(obj) = type_def {
                        let interface_values: Vec<_> = obj
                            .implements_interfaces
                            .iter()
                            .filter_map(|iface_name| ctx.schema.type_by_name(iface_name))
                            .map(|t| resolve_type_definition(t, &field.selections, ctx, arena))
                            .collect();
                        Value::Array(arena.alloc_slice_fill_iter(interface_values))
                    } else {
                        Value::Null
                    }
                }
                "possibleTypes" => {
                    if let TypeDefinition::Interface(_) | TypeDefinition::Union(_) = type_def {
                        let possible_types: Vec<Value<'a>> = ctx
                            .metadata
                            .possible_types
                            .get_possible_types(type_def.name())
                            .into_iter()
                            .filter(|v| v != type_def.name())
                            .filter_map(|name| ctx.schema.type_by_name(name.as_str()))
                            .map(|t| resolve_type_definition(t, &field.selections, ctx, arena))
                            .collect();
                        Value::Array(arena.alloc_slice_fill_iter(possible_types))
                    } else {
                        Value::Null
                    }
                }
                "enumValues" => {
                    if let TypeDefinition::Enum(enum_type) = type_def {
                        let include_deprecated = field
                            .arguments
                            .as_ref()
                            .and_then(|a| a.get_argument("includeDeprecated"))
                            .and_then(|v| match v {
                                AstValue::Boolean(b) => Some(*b),
                                AstValue::Variable(var_name) => {
                                    resolve_boolean_variable(var_name.as_str(), &ctx.variables)
                                }
                                _ => None,
                            })
                            .unwrap_or(false);

                        let enum_values: Vec<_> = enum_type
                            .values
                            .iter()
                            .filter(|v| include_deprecated || !is_deprecated_enum(v))
                            .map(|v| resolve_enum_value(v, &field.selections, arena))
                            .collect();
                        Value::Array(arena.alloc_slice_fill_iter(enum_values))
                    } else {
                        Value::Null
                    }
                }
                "inputFields" => match type_def {
                    TypeDefinition::InputObject(io) => {
                        let fields_values: Vec<_> = io
                            .fields
                            .iter()
                            .map(|f| resolve_input_value(f, &field.selections, ctx, arena))
                            .collect();
                        Value::Array(arena.alloc_slice_fill_iter(fields_values))
                    }
                    _ => Value::Null,
                },
                "ofType" => Value::Null,
                "__typename" => Value::String("__Type"),
                _ => Value::Null,
            };
            type_data.push((field.selection_identifier(), value));
        } else if let SelectionItem::InlineFragment(_) = item {
            let selection_items = item.selections();
            if let Some(selection_items) = selection_items {
                let new_data =
                    resolve_type_definition_selections(type_def, selection_items, ctx, arena);
                type_data.extend(new_data);
            }
        }
    }
    type_data
}
fn resolve_wrapper_type<'exec, 'a>(
    kind: &'exec str,
    inner_type: &'exec Type,
    selections: &'exec SelectionSet,
    ctx: &'exec IntrospectionContext,
    arena: &'a Bump,
) -> Value<'a> {
    into_slots(
        resolve_wrapper_type_selections(kind, inner_type, &selections.items, ctx, arena),
        selections,
        arena,
    )
}

fn resolve_wrapper_type_selections<'exec, 'a>(
    kind: &'exec str,
    inner_type: &'exec Type,
    selection_items: &'exec Vec<SelectionItem>,
    ctx: &'exec IntrospectionContext,
    arena: &'a Bump,
) -> Vec<(&'exec str, Value<'a>)> {
    let mut type_data = Vec::with_capacity(selection_items.len());
    for item in selection_items {
        if let SelectionItem::Field(field) = item {
            let value = match field.name.as_str() {
                "kind" => str_value(arena, kind),
                "name" => Value::Null,
                "ofType" => resolve_type(inner_type, &field.selections, ctx, arena),
                "__typename" => Value::String("__Type"),
                _ => Value::Null,
            };
            type_data.push((field.selection_identifier(), value));
        } else if let SelectionItem::InlineFragment(_) = item {
            let selection_items = item.selections();
            if let Some(selection_items) = selection_items {
                let new_data =
                    resolve_wrapper_type_selections(kind, inner_type, selection_items, ctx, arena);
                type_data.extend(new_data);
            }
        }
    }
    type_data
}

fn resolve_type<'exec, 'a>(
    t: &'exec Type,
    selections: &'exec SelectionSet,
    ctx: &'exec IntrospectionContext,
    arena: &'a Bump,
) -> Value<'a> {
    match t {
        Type::NamedType(name) => {
            let type_def = ctx.schema.type_by_name(name).unwrap_or_else(|| {
                panic!(
                    "Type '{}' not found in the schema unexpectedly during introspection",
                    name
                );
            });
            resolve_type_definition(type_def, selections, ctx, arena)
        }
        Type::ListType(inner_t) => resolve_wrapper_type("LIST", inner_t, selections, ctx, arena),
        Type::NonNullType(inner_t) => {
            resolve_wrapper_type("NON_NULL", inner_t, selections, ctx, arena)
        }
    }
}

fn resolve_directive<'exec, 'a>(
    d: &'exec DirectiveDefinition,
    selections: &'exec SelectionSet,
    ctx: &'exec IntrospectionContext,
    arena: &'a Bump,
) -> Value<'a> {
    into_slots(
        resolve_directive_selections(d, &selections.items, ctx, arena),
        selections,
        arena,
    )
}

fn resolve_directive_selections<'exec, 'a>(
    d: &'exec DirectiveDefinition,
    selection_items: &'exec Vec<SelectionItem>,
    ctx: &'exec IntrospectionContext,
    arena: &'a Bump,
) -> Vec<(&'exec str, Value<'a>)> {
    let mut directive_data = Vec::with_capacity(selection_items.len());
    for item in selection_items {
        if let SelectionItem::Field(field) = item {
            let value = match field.name.as_str() {
                "name" => str_value(arena, d.name.as_str()),
                "description" => d
                    .description
                    .as_ref()
                    .map_or(Value::Null, |s| str_value(arena, s.as_str())),
                "locations" => {
                    let locs: Vec<_> = d
                        .locations
                        .iter()
                        .map(|l| str_value(arena, l.as_str()))
                        .collect();
                    Value::Array(arena.alloc_slice_fill_iter(locs))
                }
                "args" => {
                    let args: Vec<_> = d
                        .arguments
                        .iter()
                        .map(|arg| resolve_input_value(arg, &field.selections, ctx, arena))
                        .collect();
                    Value::Array(arena.alloc_slice_fill_iter(args))
                }
                "isRepeatable" => Value::Bool(d.repeatable),
                "__typename" => Value::String("__Directive"),
                _ => Value::Null,
            };
            directive_data.push((field.selection_identifier(), value));
        } else if let SelectionItem::InlineFragment(_) = item {
            let selection_items = item.selections();
            if let Some(selection_items) = selection_items {
                let new_data = resolve_directive_selections(d, selection_items, ctx, arena);
                directive_data.extend(new_data);
            }
        }
    }
    directive_data
}

fn resolve_schema_field<'exec, 'a>(
    field: &'exec FieldSelection,
    ctx: &'exec IntrospectionContext,
    arena: &'a Bump,
) -> Value<'a> {
    into_slots(
        resolve_schema_selections(&field.selections.items, ctx, arena),
        &field.selections,
        arena,
    )
}

fn resolve_schema_selections<'exec, 'a>(
    items: &'exec Vec<SelectionItem>,
    ctx: &'exec IntrospectionContext,
    arena: &'a Bump,
) -> Vec<(&'exec str, Value<'a>)> {
    let mut schema_data = Vec::with_capacity(items.len());

    for item in items {
        if let SelectionItem::Field(inner_field) = item {
            let value = match inner_field.name.as_str() {
                "description" => Value::Null,
                "types" => {
                    let types: Vec<Value<'_>> = ctx
                        .schema
                        .type_map()
                        .values()
                        .map(|t| resolve_type_definition(t, &inner_field.selections, ctx, arena))
                        .collect();
                    Value::Array(arena.alloc_slice_fill_iter(types))
                }
                "queryType" => {
                    let query_type = ctx
                        .metadata
                        .query_type_name
                        .as_ref()
                        .and_then(|name| ctx.schema.type_by_name(name))
                        // SAFETY: The query type is guaranteed to exist,
                        // every schema has a query type.
                        .expect("invariant violation: query type is guaranteed to exist because every schema must have a query type");
                    resolve_type_definition(query_type, &inner_field.selections, ctx, arena)
                }
                "mutationType" => ctx
                    .schema
                    .mutation_type_name()
                    .and_then(|name| ctx.schema.type_by_name(name))
                    .map_or(Value::Null, |t| {
                        resolve_type_definition(t, &inner_field.selections, ctx, arena)
                    }),
                "subscriptionType" => ctx
                    .schema
                    .subscription_type_name()
                    .and_then(|name| ctx.schema.type_by_name(name))
                    .map_or(Value::Null, |t| {
                        resolve_type_definition(t, &inner_field.selections, ctx, arena)
                    }),
                "directives" => {
                    let directives: Vec<Value<'_>> = ctx
                        .schema
                        .definitions
                        .iter()
                        .filter_map(|d| match d {
                            Definition::DirectiveDefinition(d) => Some(d),
                            _ => None,
                        })
                        .map(|d| resolve_directive(d, &inner_field.selections, ctx, arena))
                        .collect();
                    Value::Array(arena.alloc_slice_fill_iter(directives))
                }
                "__typename" => Value::String("__Schema"),
                _ => Value::Null,
            };
            schema_data.push((inner_field.selection_identifier(), value));
        } else if let SelectionItem::FragmentSpread(_) = item {
            let selection_items = item.selections();
            if let Some(selection_items) = selection_items {
                let new_data = resolve_schema_selections(selection_items, ctx, arena);
                schema_data.extend(new_data);
            }
        }
    }
    schema_data
}

pub fn resolve_introspection<'exec, 'a>(
    operation_definition: &'exec OperationDefinition,
    ctx: &'exec IntrospectionContext,
    arena: &'a Bump,
) -> Value<'a> {
    let root_selection_set = &operation_definition.selection_set;
    let root_type_name = ctx
        .metadata
        .expect_root_type_name(operation_definition.operation_kind.as_ref());

    into_slots(
        resolve_root_introspection_selections(
            root_type_name,
            &root_selection_set.items,
            ctx,
            arena,
        ),
        root_selection_set,
        arena,
    )
}

fn resolve_root_introspection_selections<'exec, 'a>(
    root_type_name: &'exec str,
    items: &'exec Vec<SelectionItem>,
    ctx: &'exec IntrospectionContext,
    arena: &'a Bump,
) -> Vec<(&'exec str, Value<'a>)> {
    let mut data = Vec::with_capacity(items.len());
    for item in items {
        if let SelectionItem::Field(field) = item {
            let value = match field.name.as_str() {
                "__schema" => resolve_schema_field(field, ctx, arena),
                "__type" => {
                    if let Some(args) = &field.arguments {
                        let type_value = match args.get_argument("name") {
                            Some(AstValue::String(type_name)) => {
                                ctx.schema.type_by_name(type_name).map_or(Value::Null, |t| {
                                    resolve_type_definition(t, &field.selections, ctx, arena)
                                })
                            }
                            Some(AstValue::Variable(var_name)) => {
                                if let Some(var_value) =
                                    resolve_str_variable(var_name.as_str(), &ctx.variables)
                                {
                                    ctx.schema.type_by_name(var_value).map_or(Value::Null, |t| {
                                        resolve_type_definition(t, &field.selections, ctx, arena)
                                    })
                                } else {
                                    Value::Null
                                }
                            }

                            _ => Value::Null,
                        };

                        type_value
                    } else {
                        Value::Null
                    }
                }
                "__typename" => str_value(arena, root_type_name),
                _ => Value::Null,
            };
            data.push((field.selection_identifier(), value));
        } else if let SelectionItem::InlineFragment(_) = item {
            let selection_items = item.selections();
            if let Some(selection_items) = selection_items {
                let new_data = resolve_root_introspection_selections(
                    root_type_name,
                    selection_items,
                    ctx,
                    arena,
                );
                data.extend(new_data);
            }
        }
    }
    data
}
