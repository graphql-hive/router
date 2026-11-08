use std::borrow::Cow;
use std::sync::Arc;

use graphql_tools::parser::query::Value as QueryValue;
use graphql_tools::static_graphql::schema::{
    Definition, Directive, DirectiveDefinition, Document, EnumValue, Field, InputValue,
    InterfaceType as TypeDefinitionInterface, ObjectType as TypeDefinitionObject, Type,
    TypeDefinition,
};

use crate::query_planner::ast::{
    operation::OperationDefinition,
    selection_item::SelectionItem,
    selection_set::{FieldSelection, SelectionSet},
    value::Value as AstValue,
};
use sonic_rs::JsonValueTrait;

use crate::executor::execution::plan::CoerceVariablesPayload;
use crate::executor::introspection::schema::SchemaMetadata;
use crate::executor::json_writer::write_and_escape_string;

pub struct IntrospectionContext {
    pub query: Option<Arc<OperationDefinition>>,
    pub schema: Arc<Document>,
    pub metadata: Arc<SchemaMetadata>,
    pub variables: Arc<CoerceVariablesPayload>,
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

fn kind_to_str(type_def: &TypeDefinition) -> Cow<'static, str> {
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

/// The JSON the resolver writes into
struct Out {
    buffer: Vec<u8>,
}

impl Out {
    /// A key of the object being written. `first` is that object's: whether nothing was
    /// written in it yet. Response keys never need escaping.
    fn key(&mut self, first: &mut bool, key: &str) {
        if !std::mem::take(first) {
            self.buffer.push(b',');
        }
        self.buffer.push(b'"');
        self.buffer.extend_from_slice(key.as_bytes());
        self.buffer.extend_from_slice(b"\":");
    }

    fn string(&mut self, value: &str) {
        write_and_escape_string(&mut self.buffer, value);
    }

    fn maybe_string(&mut self, value: Option<&str>) {
        match value {
            Some(value) => self.string(value),
            None => self.null(),
        }
    }

    fn bool(&mut self, value: bool) {
        self.buffer
            .extend_from_slice(if value { b"true" } else { b"false" });
    }

    fn null(&mut self) {
        self.buffer.extend_from_slice(b"null");
    }

    /// An object: `fields` writes its fields with [`Out::key`]
    fn object(&mut self, fields: impl FnOnce(&mut Self, &mut bool)) {
        self.buffer.push(b'{');
        fields(self, &mut true);
        self.buffer.push(b'}');
    }

    /// A list of `items`, each written by `item`
    fn list<T>(&mut self, items: impl IntoIterator<Item = T>, mut item: impl FnMut(&mut Self, T)) {
        self.buffer.push(b'[');
        for (index, value) in items.into_iter().enumerate() {
            if index > 0 {
                self.buffer.push(b',');
            }
            item(self, value);
        }
        self.buffer.push(b']');
    }
}

/// Whether `includeDeprecated` is true on `field`
fn include_deprecated(field: &FieldSelection, ctx: &IntrospectionContext) -> bool {
    field
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
        .unwrap_or(false)
}

fn resolve_input_value(
    out: &mut Out,
    iv: &InputValue,
    selections: &SelectionSet,
    ctx: &IntrospectionContext,
) {
    out.object(|out, first| resolve_input_value_selections(out, first, iv, &selections.items, ctx));
}

fn resolve_input_value_selections(
    out: &mut Out,
    first: &mut bool,
    iv: &InputValue,
    selection_items: &[SelectionItem],
    ctx: &IntrospectionContext,
) {
    for item in selection_items {
        if let SelectionItem::Field(field) = item {
            out.key(first, field.selection_identifier());
            match field.name.as_str() {
                "name" => out.string(&iv.name),
                "description" => out.maybe_string(iv.description.as_deref()),
                "type" => resolve_type(out, &iv.value_type, &field.selections, ctx),
                // TODO: support default values
                "defaultValue" => match &iv.default_value {
                    Some(ast) => out.string(&ast.to_string()),
                    None => out.null(),
                },
                "isDeprecated" => out.bool(is_deprecated(&iv.directives)),
                "deprecationReason" => out.maybe_string(get_deprecation_reason(&iv.directives)),
                "__typename" => out.string("__InputValue"),
                _ => out.null(),
            }
        } else if let Some(selection_items) = inline_selections(item) {
            resolve_input_value_selections(out, first, iv, selection_items, ctx);
        }
    }
}

/// The selections of an inline fragment
fn inline_selections(item: &SelectionItem) -> Option<&Vec<SelectionItem>> {
    match item {
        SelectionItem::InlineFragment(_) => item.selections(),
        _ => None,
    }
}

fn resolve_field(out: &mut Out, f: &Field, selections: &SelectionSet, ctx: &IntrospectionContext) {
    out.object(|out, first| resolve_field_selections(out, first, f, &selections.items, ctx));
}

fn resolve_field_selections(
    out: &mut Out,
    first: &mut bool,
    f: &Field,
    selection_items: &[SelectionItem],
    ctx: &IntrospectionContext,
) {
    for item in selection_items {
        if let SelectionItem::Field(field) = item {
            out.key(first, field.selection_identifier());
            match field.name.as_str() {
                "name" => out.string(&f.name),
                "description" => out.maybe_string(f.description.as_deref()),
                "args" => out.list(&f.arguments, |out, arg| {
                    resolve_input_value(out, arg, &field.selections, ctx)
                }),
                "type" => resolve_type(out, &f.field_type, &field.selections, ctx),
                "isDeprecated" => out.bool(is_deprecated(&f.directives)),
                "deprecationReason" => out.maybe_string(get_deprecation_reason(&f.directives)),
                "__typename" => out.string("__Field"),
                _ => out.null(),
            }
        } else if let Some(selection_items) = inline_selections(item) {
            resolve_field_selections(out, first, f, selection_items, ctx);
        }
    }
}

fn resolve_enum_value(out: &mut Out, ev: &EnumValue, selections: &SelectionSet) {
    out.object(|out, first| resolve_enum_value_selections(out, first, ev, &selections.items));
}

fn resolve_enum_value_selections(
    out: &mut Out,
    first: &mut bool,
    ev: &EnumValue,
    selection_items: &[SelectionItem],
) {
    for item in selection_items {
        if let SelectionItem::Field(field) = item {
            out.key(first, field.selection_identifier());
            match field.name.as_str() {
                "name" => out.string(&ev.name),
                "description" => out.maybe_string(ev.description.as_deref()),
                "isDeprecated" => out.bool(is_deprecated_enum(ev)),
                "deprecationReason" => out.maybe_string(get_deprecation_reason(&ev.directives)),
                "__typename" => out.string("__EnumValue"),
                _ => out.null(),
            }
        } else if let Some(selection_items) = inline_selections(item) {
            resolve_enum_value_selections(out, first, ev, selection_items);
        }
    }
}

fn resolve_type_definition(
    out: &mut Out,
    type_def: &TypeDefinition,
    selections: &SelectionSet,
    ctx: &IntrospectionContext,
) {
    out.object(|out, first| {
        resolve_type_definition_selections(out, first, type_def, &selections.items, ctx)
    });
}

fn resolve_type_definition_selections(
    out: &mut Out,
    first: &mut bool,
    type_def: &TypeDefinition,
    selection_items: &[SelectionItem],
    ctx: &IntrospectionContext,
) {
    for item in selection_items {
        if let SelectionItem::Field(field) = item {
            out.key(first, field.selection_identifier());
            match field.name.as_str() {
                "kind" => out.string(&kind_to_str(type_def)),
                "name" => out.string(type_def.name()),
                "description" => out.maybe_string(
                    match type_def {
                        TypeDefinition::Scalar(s) => s.description.as_ref(),
                        TypeDefinition::Object(o) => o.description.as_ref(),
                        TypeDefinition::Interface(i) => i.description.as_ref(),
                        TypeDefinition::Union(u) => u.description.as_ref(),
                        TypeDefinition::Enum(e) => e.description.as_ref(),
                        TypeDefinition::InputObject(io) => io.description.as_ref(),
                    }
                    .map(String::as_str),
                ),
                "specifiedByURL" => match type_def {
                    TypeDefinition::Scalar(scalar) => {
                        out.maybe_string(get_specified_by_url(&scalar.directives))
                    }
                    _ => out.null(),
                },
                "isOneOf" => match type_def {
                    TypeDefinition::InputObject(type_def) => {
                        out.bool(is_one_of(&type_def.directives))
                    }
                    _ => out.null(),
                },
                "fields" => match type_def {
                    TypeDefinition::Object(TypeDefinitionObject { fields, .. })
                    | TypeDefinition::Interface(TypeDefinitionInterface { fields, .. }) => {
                        let include_deprecated = include_deprecated(field, ctx);
                        out.list(
                            fields.iter().filter(|f| {
                                !f.name.starts_with("__")
                                    && (include_deprecated || !is_deprecated(&f.directives))
                            }),
                            |out, f| resolve_field(out, f, &field.selections, ctx),
                        )
                    }
                    _ => out.null(),
                },
                "interfaces" => match type_def {
                    TypeDefinition::Object(obj) => out.list(
                        obj.implements_interfaces
                            .iter()
                            .filter_map(|iface_name| ctx.schema.type_by_name(iface_name)),
                        |out, t| resolve_type_definition(out, t, &field.selections, ctx),
                    ),
                    _ => out.null(),
                },
                "possibleTypes" => match type_def {
                    TypeDefinition::Interface(_) | TypeDefinition::Union(_) => out.list(
                        ctx.metadata
                            .possible_types
                            .get_possible_types(type_def.name())
                            .into_iter()
                            .filter(|v| v != type_def.name())
                            .filter_map(|name| ctx.schema.type_by_name(name.as_str())),
                        |out, t| resolve_type_definition(out, t, &field.selections, ctx),
                    ),
                    _ => out.null(),
                },
                "enumValues" => match type_def {
                    TypeDefinition::Enum(enum_type) => {
                        let include_deprecated = include_deprecated(field, ctx);
                        out.list(
                            enum_type
                                .values
                                .iter()
                                .filter(|v| include_deprecated || !is_deprecated_enum(v)),
                            |out, v| resolve_enum_value(out, v, &field.selections),
                        )
                    }
                    _ => out.null(),
                },
                "inputFields" => match type_def {
                    TypeDefinition::InputObject(io) => out.list(&io.fields, |out, f| {
                        resolve_input_value(out, f, &field.selections, ctx)
                    }),
                    _ => out.null(),
                },
                "__typename" => out.string("__Type"),
                // "ofType" of a named type, and anything else
                _ => out.null(),
            }
        } else if let Some(selection_items) = inline_selections(item) {
            resolve_type_definition_selections(out, first, type_def, selection_items, ctx);
        }
    }
}

fn resolve_wrapper_type(
    out: &mut Out,
    kind: &str,
    inner_type: &Type,
    selections: &SelectionSet,
    ctx: &IntrospectionContext,
) {
    out.object(|out, first| {
        resolve_wrapper_type_selections(out, first, kind, inner_type, &selections.items, ctx)
    });
}

fn resolve_wrapper_type_selections(
    out: &mut Out,
    first: &mut bool,
    kind: &str,
    inner_type: &Type,
    selection_items: &[SelectionItem],
    ctx: &IntrospectionContext,
) {
    for item in selection_items {
        if let SelectionItem::Field(field) = item {
            out.key(first, field.selection_identifier());
            match field.name.as_str() {
                "kind" => out.string(kind),
                "ofType" => resolve_type(out, inner_type, &field.selections, ctx),
                "__typename" => out.string("__Type"),
                // "name" of a wrapper type, and anything else
                _ => out.null(),
            }
        } else if let Some(selection_items) = inline_selections(item) {
            resolve_wrapper_type_selections(out, first, kind, inner_type, selection_items, ctx);
        }
    }
}

fn resolve_type(out: &mut Out, t: &Type, selections: &SelectionSet, ctx: &IntrospectionContext) {
    match t {
        Type::NamedType(name) => {
            let type_def = ctx.schema.type_by_name(name).unwrap_or_else(|| {
                panic!(
                    "Type '{}' not found in the schema unexpectedly during introspection",
                    name
                );
            });
            resolve_type_definition(out, type_def, selections, ctx)
        }
        Type::ListType(inner_t) => resolve_wrapper_type(out, "LIST", inner_t, selections, ctx),
        Type::NonNullType(inner_t) => {
            resolve_wrapper_type(out, "NON_NULL", inner_t, selections, ctx)
        }
    }
}

fn resolve_directive(
    out: &mut Out,
    d: &DirectiveDefinition,
    selections: &SelectionSet,
    ctx: &IntrospectionContext,
) {
    out.object(|out, first| resolve_directive_selections(out, first, d, &selections.items, ctx));
}

fn resolve_directive_selections(
    out: &mut Out,
    first: &mut bool,
    d: &DirectiveDefinition,
    selection_items: &[SelectionItem],
    ctx: &IntrospectionContext,
) {
    for item in selection_items {
        if let SelectionItem::Field(field) = item {
            out.key(first, field.selection_identifier());
            match field.name.as_str() {
                "name" => out.string(&d.name),
                "description" => out.maybe_string(d.description.as_deref()),
                "locations" => out.list(&d.locations, |out, l| out.string(l.as_str())),
                "args" => out.list(&d.arguments, |out, arg| {
                    resolve_input_value(out, arg, &field.selections, ctx)
                }),
                "isRepeatable" => out.bool(d.repeatable),
                "__typename" => out.string("__Directive"),
                _ => out.null(),
            }
        } else if let Some(selection_items) = inline_selections(item) {
            resolve_directive_selections(out, first, d, selection_items, ctx);
        }
    }
}

fn resolve_schema_selections(
    out: &mut Out,
    first: &mut bool,
    items: &[SelectionItem],
    ctx: &IntrospectionContext,
) {
    for item in items {
        if let SelectionItem::Field(inner_field) = item {
            out.key(first, inner_field.selection_identifier());
            let selections = &inner_field.selections;
            match inner_field.name.as_str() {
                "types" => out.list(ctx.schema.type_map().values(), |out, t| {
                    resolve_type_definition(out, t, selections, ctx)
                }),
                "queryType" => {
                    let query_type = ctx
                        .metadata
                        .query_type_name
                        .as_ref()
                        .and_then(|name| ctx.schema.type_by_name(name))
                        // SAFETY: The query type is guaranteed to exist,
                        // every schema has a query type.
                        .expect("invariant violation: query type is guaranteed to exist because every schema must have a query type");
                    resolve_type_definition(out, query_type, selections, ctx)
                }
                "mutationType" => match ctx
                    .schema
                    .mutation_type_name()
                    .and_then(|name| ctx.schema.type_by_name(name))
                {
                    Some(t) => resolve_type_definition(out, t, selections, ctx),
                    None => out.null(),
                },
                "subscriptionType" => match ctx
                    .schema
                    .subscription_type_name()
                    .and_then(|name| ctx.schema.type_by_name(name))
                {
                    Some(t) => resolve_type_definition(out, t, selections, ctx),
                    None => out.null(),
                },
                "directives" => out.list(
                    ctx.schema.definitions.iter().filter_map(|d| match d {
                        Definition::DirectiveDefinition(d) => Some(d),
                        _ => None,
                    }),
                    |out, d| resolve_directive(out, d, selections, ctx),
                ),
                "__typename" => out.string("__Schema"),
                // "description", and anything else
                _ => out.null(),
            }
        } else if let SelectionItem::FragmentSpread(_) = item {
            if let Some(selection_items) = item.selections() {
                resolve_schema_selections(out, first, selection_items, ctx);
            }
        }
    }
}

/// The introspection part of an operation, as the JSON object of its `data`
pub fn resolve_introspection(
    operation_definition: &OperationDefinition,
    ctx: &IntrospectionContext,
) -> Vec<u8> {
    let root_type_name = ctx
        .metadata
        .expect_root_type_name(operation_definition.operation_kind.as_ref());
    let mut out = Out { buffer: Vec::new() };
    out.object(|out, first| {
        resolve_root_introspection_selections(
            out,
            first,
            root_type_name,
            &operation_definition.selection_set.items,
            ctx,
        )
    });
    out.buffer
}

fn resolve_root_introspection_selections(
    out: &mut Out,
    first: &mut bool,
    root_type_name: &str,
    items: &[SelectionItem],
    ctx: &IntrospectionContext,
) {
    for item in items {
        if let SelectionItem::Field(field) = item {
            out.key(first, field.selection_identifier());
            match field.name.as_str() {
                "__schema" => out.object(|out, first| {
                    resolve_schema_selections(out, first, &field.selections.items, ctx)
                }),
                "__type" => {
                    let type_name = match field
                        .arguments
                        .as_ref()
                        .and_then(|args| args.get_argument("name"))
                    {
                        Some(AstValue::String(type_name)) => Some(type_name.as_str()),
                        Some(AstValue::Variable(var_name)) => {
                            resolve_str_variable(var_name.as_str(), &ctx.variables)
                        }
                        _ => None,
                    };
                    match type_name.and_then(|name| ctx.schema.type_by_name(name)) {
                        Some(t) => resolve_type_definition(out, t, &field.selections, ctx),
                        None => out.null(),
                    }
                }
                "__typename" => out.string(root_type_name),
                _ => out.null(),
            }
        } else if let Some(selection_items) = inline_selections(item) {
            resolve_root_introspection_selections(out, first, root_type_name, selection_items, ctx);
        }
    }
}
