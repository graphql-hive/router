use std::borrow::Cow;
use std::collections::HashMap;

use graphql_parser::query::Value as QueryValue;
use graphql_parser::schema::{
    Definition, Directive, DirectiveDefinition, Document, EnumValue, Field, InputValue, Type,
    TypeDefinition,
};

use hive_router_query_planner::ast::{
    operation::OperationDefinition,
    selection_item::SelectionItem,
    selection_set::{FieldSelection, SelectionSet},
    value::Value as AstValue,
};
use hive_router_query_planner::state::supergraph_state::OperationKind;
use tracing::trace;

use crate::introspection::schema::SchemaMetadata;
use crate::response::value::Value;

pub struct IntrospectionContext<'exec, 'schema> {
    pub query: Option<&'exec OperationDefinition>,
    pub schema: &'exec Document<'schema, String>,
    pub metadata: &'exec SchemaMetadata,
}

fn get_deprecation_reason<'exec, 'schema>(
    directives: &'exec [Directive<'schema, String>],
) -> Option<&'exec str> {
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

fn is_deprecated<'exec, 'schema: 'exec>(directives: &'exec [Directive<'schema, String>]) -> bool {
    directives.iter().any(|d| d.name == "deprecated")
}

fn is_deprecated_enum<'exec, 'schema: 'exec>(enum_val: &'exec EnumValue<'schema, String>) -> bool {
    is_deprecated(&enum_val.directives)
}

fn kind_to_str<'exec, 'schema: 'exec>(
    type_def: &'exec TypeDefinition<'schema, String>,
) -> Cow<'exec, str> {
    (match type_def {
        TypeDefinition::Scalar(_) => "SCALAR",
        TypeDefinition::Object(_) => "OBJECT",
        TypeDefinition::Interface(_) => "INTERFACE",
        TypeDefinition::Union(_) => "UNION",
        TypeDefinition::Enum(_) => "ENUM",
        TypeDefinition::InputObject(_) => "INPUT_OBJECT",
    })
    .into()
}

fn resolve_input_value<'exec, 'schema: 'exec>(
    iv: &'exec InputValue<'schema, String>,
    selections: &'exec SelectionSet,
    ctx: &'exec IntrospectionContext<'exec, 'schema>,
) -> Value<'exec> {
    let mut iv_data = resolve_input_value_selections(iv, &selections.items, ctx);
    iv_data.sort_by_key(|(k, _)| *k);
    Value::Object(iv_data)
}

fn resolve_input_value_selections<'exec, 'schema: 'exec>(
    iv: &'exec InputValue<'schema, String>,
    selection_items: &'exec Vec<SelectionItem>,
    ctx: &'exec IntrospectionContext<'exec, 'schema>,
) -> Vec<(&'exec str, Value<'exec>)> {
    let mut iv_data: Vec<(&str, Value<'_>)> = Vec::with_capacity(selection_items.len());
    for item in selection_items {
        if let SelectionItem::Field(field) = item {
            let value = match field.name.as_str() {
                "name" => Value::String(iv.name.as_str().into()),
                "description" => iv
                    .description
                    .as_ref()
                    .map_or(Value::Null, |s| Value::String(s.into())),
                "type" => resolve_type(&iv.value_type, &field.selections, ctx),
                "defaultValue" => Value::Null, // TODO: support default values
                "__typename" => Value::String("__InputValue".into()),
                _ => Value::Null,
            };
            iv_data.push((field.selection_identifier(), value));
        } else if let SelectionItem::InlineFragment(_) = item {
            let selection_items = item.selections();
            if let Some(selection_items) = selection_items {
                let new_data = resolve_input_value_selections(iv, selection_items, ctx);
                iv_data.extend(new_data);
            }
        }
    }
    iv_data
}

fn resolve_field<'exec, 'schema: 'exec>(
    f: &'exec Field<'schema, String>,
    selections: &'exec SelectionSet,
    ctx: &'exec IntrospectionContext<'exec, 'schema>,
) -> Value<'exec> {
    let mut field_data = resolve_field_selections(f, &selections.items, ctx);
    field_data.sort_by_key(|(k, _)| *k);
    Value::Object(field_data)
}

fn resolve_field_selections<'exec, 'schema: 'exec>(
    f: &'exec Field<'schema, String>,
    selection_items: &'exec Vec<SelectionItem>,
    ctx: &'exec IntrospectionContext<'exec, 'schema>,
) -> Vec<(&'exec str, Value<'exec>)> {
    let mut field_data = Vec::with_capacity(selection_items.len());
    for item in selection_items {
        if let SelectionItem::Field(field) = item {
            let value = match field.name.as_str() {
                "name" => Value::String(f.name.as_str().into()),
                "description" => f
                    .description
                    .as_ref()
                    .map_or(Value::Null, |s| Value::String(s.into())),
                "args" => {
                    let args: Vec<_> = f
                        .arguments
                        .iter()
                        .map(|arg| resolve_input_value(arg, &field.selections, ctx))
                        .collect();
                    Value::Array(args)
                }
                "type" => resolve_type(&f.field_type, &field.selections, ctx),
                "isDeprecated" => Value::Bool(is_deprecated(&f.directives)),
                "deprecationReason" => get_deprecation_reason(&f.directives)
                    .map_or(Value::Null, |s| Value::String(s.into())),
                "__typename" => Value::String("__Field".into()),
                _ => Value::Null,
            };
            field_data.push((field.selection_identifier(), value));
        } else if let SelectionItem::InlineFragment(_) = item {
            let selection_items = item.selections();
            if let Some(selection_items) = selection_items {
                let new_data = resolve_field_selections(f, selection_items, ctx);
                field_data.extend(new_data);
            }
        }
    }
    field_data
}

fn resolve_enum_value<'exec, 'schema: 'exec>(
    ev: &'exec EnumValue<'schema, String>,
    selections: &'exec SelectionSet,
) -> Value<'exec> {
    let mut ev_data = resolve_enum_value_selections(ev, &selections.items);
    ev_data.sort_by_key(|(k, _)| *k);
    Value::Object(ev_data)
}

fn resolve_enum_value_selections<'exec, 'schema: 'exec>(
    ev: &'exec EnumValue<'schema, String>,
    selection_items: &'exec Vec<SelectionItem>,
) -> Vec<(&'exec str, Value<'exec>)> {
    let mut ev_data = Vec::with_capacity(selection_items.len());
    for item in selection_items {
        if let SelectionItem::Field(field) = item {
            let value = match field.name.as_str() {
                "name" => Value::String(ev.name.as_str().into()),
                "description" => ev
                    .description
                    .as_ref()
                    .map_or(Value::Null, |s| Value::String(s.into())),
                "isDeprecated" => Value::Bool(is_deprecated_enum(ev)),
                "deprecationReason" => get_deprecation_reason(&ev.directives)
                    .map_or(Value::Null, |s| Value::String(s.into())),
                "__typename" => Value::String("__EnumValue".into()),
                _ => Value::Null,
            };
            ev_data.push((field.selection_identifier(), value));
        } else if let SelectionItem::InlineFragment(_) = item {
            let selection_items = item.selections();
            if let Some(selection_items) = selection_items {
                let new_data = resolve_enum_value_selections(ev, selection_items);
                ev_data.extend(new_data);
            }
        }
    }
    ev_data
}

fn resolve_type_definition<'exec, 'schema: 'exec>(
    type_def: &'exec TypeDefinition<'schema, String>,
    selections: &'exec SelectionSet,
    ctx: &'exec IntrospectionContext<'exec, 'schema>,
) -> Value<'exec> {
    let mut type_data = resolve_type_definition_selections(type_def, &selections.items, ctx);
    type_data.sort_by_key(|(k, _)| *k);
    Value::Object(type_data)
}

fn resolve_type_definition_selections<'exec, 'schema: 'exec>(
    type_def: &'exec TypeDefinition<'schema, String>,
    selection_items: &'exec Vec<SelectionItem>,
    ctx: &'exec IntrospectionContext<'exec, 'schema>,
) -> Vec<(&'exec str, Value<'exec>)> {
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
                .map(|s| Value::String(s.into()))
                .unwrap_or(Value::Null),
                "description" => match type_def {
                    TypeDefinition::Scalar(s) => s.description.as_ref(),
                    TypeDefinition::Object(o) => o.description.as_ref(),
                    TypeDefinition::Interface(i) => i.description.as_ref(),
                    TypeDefinition::Union(u) => u.description.as_ref(),
                    TypeDefinition::Enum(e) => e.description.as_ref(),
                    TypeDefinition::InputObject(io) => io.description.as_ref(),
                }
                .map_or(Value::Null, |s| Value::String(s.into())),
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
                            .and_then(|v| {
                                if let AstValue::Boolean(b) = v {
                                    Some(*b)
                                } else {
                                    None
                                }
                            })
                            .unwrap_or(false);

                        let fields_values: Vec<Value<'exec>> = fields
                            .iter()
                            .filter(|f| {
                                !f.name.starts_with("__")
                                    && (include_deprecated || !is_deprecated(&f.directives))
                            })
                            .map(|f| resolve_field(f, &field.selections, ctx))
                            .collect();
                        Value::Array(fields_values)
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
                            .map(|t| resolve_type_definition(t, &field.selections, ctx))
                            .collect();
                        Value::Array(interface_values)
                    } else {
                        Value::Null
                    }
                }
                "possibleTypes" => {
                    if let TypeDefinition::Interface(_) | TypeDefinition::Union(_) = type_def {
                        let possible_types: Vec<Value<'exec>> = ctx
                            .metadata
                            .possible_types
                            .get_possible_types(type_def.name())
                            .into_iter()
                            .filter(|v| v != type_def.name())
                            .filter_map(|name| ctx.schema.type_by_name(name.as_str()))
                            .map(|t| resolve_type_definition(t, &field.selections, ctx))
                            .collect();
                        Value::Array(possible_types)
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
                            .and_then(|v| {
                                if let AstValue::Boolean(b) = v {
                                    Some(*b)
                                } else {
                                    None
                                }
                            })
                            .unwrap_or(false);

                        let enum_values: Vec<_> = enum_type
                            .values
                            .iter()
                            .filter(|v| include_deprecated || !is_deprecated_enum(v))
                            .map(|v| resolve_enum_value(v, &field.selections))
                            .collect();
                        Value::Array(enum_values)
                    } else {
                        Value::Null
                    }
                }
                "inputFields" => match type_def {
                    TypeDefinition::InputObject(io) => {
                        let fields_values: Vec<_> = io
                            .fields
                            .iter()
                            .map(|f| resolve_input_value(f, &field.selections, ctx))
                            .collect();
                        Value::Array(fields_values)
                    }
                    _ => Value::Null,
                },
                "ofType" => Value::Null,
                "__typename" => Value::String("__Type".into()),
                _ => Value::Null,
            };
            type_data.push((field.selection_identifier(), value));
        } else if let SelectionItem::InlineFragment(_) = item {
            let selection_items = item.selections();
            if let Some(selection_items) = selection_items {
                let new_data = resolve_type_definition_selections(type_def, selection_items, ctx);
                type_data.extend(new_data);
            }
        }
    }
    type_data
}
fn resolve_wrapper_type<'exec, 'schema: 'exec>(
    kind: &'exec str,
    inner_type: &'exec Type<'schema, String>,
    selections: &'exec SelectionSet,
    ctx: &'exec IntrospectionContext<'exec, 'schema>,
) -> Value<'exec> {
    let mut type_data = resolve_wrapper_type_selections(kind, inner_type, &selections.items, ctx);
    type_data.sort_by_key(|(k, _)| *k);
    Value::Object(type_data)
}

fn resolve_wrapper_type_selections<'exec, 'schema: 'exec>(
    kind: &'exec str,
    inner_type: &'exec Type<'schema, String>,
    selection_items: &'exec Vec<SelectionItem>,
    ctx: &'exec IntrospectionContext<'exec, 'schema>,
) -> Vec<(&'exec str, Value<'exec>)> {
    let mut type_data = Vec::with_capacity(selection_items.len());
    for item in selection_items {
        if let SelectionItem::Field(field) = item {
            let value = match field.name.as_str() {
                "kind" => Value::String(kind.into()),
                "name" => Value::Null,
                "ofType" => resolve_type(inner_type, &field.selections, ctx),
                "__typename" => Value::String("__Type".into()),
                _ => Value::Null,
            };
            type_data.push((field.selection_identifier(), value));
        } else if let SelectionItem::InlineFragment(_) = item {
            let selection_items = item.selections();
            if let Some(selection_items) = selection_items {
                let new_data =
                    resolve_wrapper_type_selections(kind, inner_type, selection_items, ctx);
                type_data.extend(new_data);
            }
        }
    }
    type_data
}

fn resolve_type<'exec, 'schema: 'exec>(
    t: &'exec Type<'schema, String>,
    selections: &'exec SelectionSet,
    ctx: &'exec IntrospectionContext<'exec, 'schema>,
) -> Value<'exec> {
    match t {
        Type::NamedType(name) => {
            let type_def = ctx.schema.type_by_name(name);
            if let Some(type_def) = type_def {
                resolve_type_definition(type_def, selections, ctx)
            } else {
                trace!(
                    "Type '{}' not found in the schema unexpectedly during introspection",
                    name
                );
                Value::Object(vec![
                    ("name", Value::String(name.as_str().into())),
                    ("__typename", Value::String("__Type".into())),
                ])
            }
        }
        Type::ListType(inner_t) => resolve_wrapper_type("LIST", inner_t, selections, ctx),
        Type::NonNullType(inner_t) => resolve_wrapper_type("NON_NULL", inner_t, selections, ctx),
    }
}

fn resolve_directive<'exec, 'schema: 'exec>(
    d: &'exec DirectiveDefinition<'schema, String>,
    selections: &'exec SelectionSet,
    ctx: &'exec IntrospectionContext<'exec, 'schema>,
) -> Value<'exec> {
    let mut directive_data = resolve_directive_selections(d, &selections.items, ctx);
    directive_data.sort_by_key(|(k, _)| *k);
    Value::Object(directive_data)
}

fn resolve_directive_selections<'exec, 'schema: 'exec>(
    d: &'exec DirectiveDefinition<'schema, String>,
    selection_items: &'exec Vec<SelectionItem>,
    ctx: &'exec IntrospectionContext<'exec, 'schema>,
) -> Vec<(&'exec str, Value<'exec>)> {
    let mut directive_data = Vec::with_capacity(selection_items.len());
    for item in selection_items {
        if let SelectionItem::Field(field) = item {
            let value = match field.name.as_str() {
                "name" => Value::String(d.name.as_str().into()),
                "description" => d
                    .description
                    .as_ref()
                    .map_or(Value::Null, |s| Value::String(s.into())),
                "locations" => {
                    let locs: Vec<_> = d
                        .locations
                        .iter()
                        .map(|l| Value::String(l.as_str().into()))
                        .collect();
                    Value::Array(locs)
                }
                "args" => {
                    let args: Vec<_> = d
                        .arguments
                        .iter()
                        .map(|arg| resolve_input_value(arg, &field.selections, ctx))
                        .collect();
                    Value::Array(args)
                }
                "isRepeatable" => Value::Bool(d.repeatable),
                "__typename" => Value::String("__Directive".into()),
                _ => Value::Null,
            };
            directive_data.push((field.selection_identifier(), value));
        } else if let SelectionItem::InlineFragment(_) = item {
            let selection_items = item.selections();
            if let Some(selection_items) = selection_items {
                let new_data = resolve_directive_selections(d, selection_items, ctx);
                directive_data.extend(new_data);
            }
        }
    }
    directive_data
}

fn resolve_schema_field<'exec, 'schema: 'exec>(
    field: &'exec FieldSelection,
    ctx: &'exec IntrospectionContext<'exec, 'schema>,
) -> Value<'exec> {
    let mut schema_data = resolve_schema_selections(&field.selections.items, ctx);

    schema_data.sort_by_key(|(k, _)| *k);
    Value::Object(schema_data)
}

fn resolve_schema_selections<'exec, 'schema: 'exec>(
    items: &'exec Vec<SelectionItem>,
    ctx: &'exec IntrospectionContext<'exec, 'schema>,
) -> Vec<(&'exec str, Value<'exec>)> {
    let mut schema_data = Vec::with_capacity(items.len());

    for item in items {
        if let SelectionItem::Field(inner_field) = item {
            let value = match inner_field.name.as_str() {
                "description" => Value::Null,
                "types" => {
                    let types = ctx
                        .schema
                        .type_map()
                        .values()
                        .map(|t| resolve_type_definition(t, &inner_field.selections, ctx))
                        .collect();
                    Value::Array(types)
                }
                "queryType" => {
                    let query_type = ctx
                        .schema
                        .type_by_name(ctx.schema.query_type_name())
                        .expect("Query type not found");
                    resolve_type_definition(query_type, &inner_field.selections, ctx)
                }
                "mutationType" => ctx
                    .schema
                    .mutation_type_name()
                    .and_then(|name| ctx.schema.type_by_name(name))
                    .map_or(Value::Null, |t| {
                        resolve_type_definition(t, &inner_field.selections, ctx)
                    }),
                "subscriptionType" => ctx
                    .schema
                    .subscription_type_name()
                    .and_then(|name| ctx.schema.type_by_name(name))
                    .map_or(Value::Null, |t| {
                        resolve_type_definition(t, &inner_field.selections, ctx)
                    }),
                "directives" => {
                    let directives = ctx
                        .schema
                        .definitions
                        .iter()
                        .filter_map(|d| match d {
                            Definition::DirectiveDefinition(d) => Some(d),
                            _ => None,
                        })
                        .map(|d| resolve_directive(d, &inner_field.selections, ctx))
                        .collect();
                    Value::Array(directives)
                }
                "__typename" => Value::String("__Schema".into()),
                _ => Value::Null,
            };
            schema_data.push((inner_field.selection_identifier(), value));
        } else if let SelectionItem::FragmentSpread(_) = item {
            let selection_items = item.selections();
            if let Some(selection_items) = selection_items {
                let new_data = resolve_schema_selections(selection_items, ctx);
                schema_data.extend(new_data);
            }
        }
    }
    schema_data
}

pub fn resolve_introspection<'exec, 'schema: 'exec>(
    operation_definition: &'exec OperationDefinition,
    ctx: &'exec IntrospectionContext<'exec, 'schema>,
) -> Value<'exec> {
    let root_selection_set = &operation_definition.selection_set;

    let root_type_name = operation_definition
        .operation_kind
        .as_ref()
        .map(|kind| match kind {
            OperationKind::Query => ctx.schema.query_type_name(),
            OperationKind::Mutation => ctx.schema.mutation_type_name().unwrap_or("Mutation"),
            OperationKind::Subscription => ctx
                .schema
                .subscription_type_name()
                .unwrap_or("Subscription"),
        })
        .unwrap_or_else(|| ctx.schema.query_type_name());

    let mut data =
        resolve_root_introspection_selections(root_type_name, &root_selection_set.items, ctx);

    data.sort_by_key(|(k, _)| *k);
    Value::Object(data)
}

fn resolve_root_introspection_selections<'exec, 'schema: 'exec>(
    root_type_name: &'schema str,
    items: &'exec Vec<SelectionItem>,
    ctx: &'exec IntrospectionContext,
) -> Vec<(&'exec str, Value<'exec>)> {
    let mut data = Vec::with_capacity(items.len());
    for item in items {
        if let SelectionItem::Field(field) = item {
            let value = match field.name.as_str() {
                "__schema" => resolve_schema_field(field, ctx),
                "__type" => {
                    if let Some(args) = &field.arguments {
                        if let Some(AstValue::String(type_name)) = args.get_argument("name") {
                            ctx.schema.type_by_name(type_name).map_or(Value::Null, |t| {
                                resolve_type_definition(t, &field.selections, ctx)
                            })
                        } else {
                            Value::Null
                        }
                    } else {
                        Value::Null
                    }
                }
                "__typename" => Value::String(root_type_name.into()),
                _ => Value::Null,
            };
            data.push((field.selection_identifier(), value));
        } else if let SelectionItem::InlineFragment(_) = item {
            let selection_items = item.selections();
            if let Some(selection_items) = selection_items {
                let new_data =
                    resolve_root_introspection_selections(root_type_name, selection_items, ctx);
                data.extend(new_data);
            }
        }
    }
    data
}

trait TypeDefinitionExtension {
    fn name(&self) -> &str;
}

/// Kamil: I couldn't use SchemaDocumentExtension of graphql_tools,
///        due to lack of strict 'lifetimes.
///        All methods of TypeDefinitionExtension and SchemaDocumentExtension
///        had `&self` and `&T` as return types.
trait SchemaDocumentExtension<'schema> {
    fn type_by_name<'a>(&'a self, name: &str) -> Option<&'a TypeDefinition<'schema, String>>;
    fn type_map<'a>(&'a self) -> HashMap<&'a str, &'a TypeDefinition<'schema, String>>;
    fn query_type_name(&self) -> &str;
    fn mutation_type_name(&self) -> Option<&str>;
    fn subscription_type_name(&self) -> Option<&str>;
}

impl<'schema> SchemaDocumentExtension<'schema> for Document<'schema, String> {
    fn type_by_name<'a>(&'a self, name: &str) -> Option<&'a TypeDefinition<'schema, String>> {
        for def in &self.definitions {
            if let Definition::TypeDefinition(type_def) = def {
                if type_def.name().eq(name) {
                    return Some(type_def);
                }
            }
        }

        None
    }

    fn type_map<'a>(&'a self) -> HashMap<&'a str, &'a TypeDefinition<'schema, String>> {
        let mut type_map = HashMap::new();

        for def in &self.definitions {
            if let Definition::TypeDefinition(type_def) = def {
                type_map.insert(type_def.name(), type_def);
            }
        }

        type_map
    }

    fn query_type_name(&self) -> &str {
        for def in &self.definitions {
            if let Definition::SchemaDefinition(schema_def) = def {
                if let Some(name) = schema_def.query.as_ref() {
                    return name.as_str();
                }
            }
        }

        root_type_name(self, "Query").unwrap_or("Query")
    }

    fn mutation_type_name(&self) -> Option<&str> {
        for def in &self.definitions {
            if let Definition::SchemaDefinition(schema_def) = def {
                if let Some(name) = schema_def.mutation.as_ref() {
                    return Some(name.as_str());
                }
            }
        }

        root_type_name(self, "Mutation")
    }

    fn subscription_type_name(&self) -> Option<&str> {
        for def in &self.definitions {
            if let Definition::SchemaDefinition(schema_def) = def {
                if let Some(name) = schema_def.subscription.as_ref() {
                    return Some(name.as_str());
                }
            }
        }

        root_type_name(self, "Subscription")
    }
}

fn root_type_name<'a, 'schema>(
    schema: &'a Document<'schema, String>,
    name: &'a str,
) -> Option<&'a str> {
    for def in &schema.definitions {
        if let Definition::TypeDefinition(TypeDefinition::Object(obj)) = def {
            if obj.name.as_str() == name {
                return Some(obj.name.as_str());
            }
        }
    }

    None
}

impl<'schema> TypeDefinitionExtension for TypeDefinition<'schema, String> {
    fn name(&self) -> &str {
        match self {
            TypeDefinition::Object(o) => &o.name,
            TypeDefinition::Interface(i) => &i.name,
            TypeDefinition::Union(u) => &u.name,
            TypeDefinition::Scalar(s) => &s.name,
            TypeDefinition::Enum(e) => &e.name,
            TypeDefinition::InputObject(i) => &i.name,
        }
    }
}
