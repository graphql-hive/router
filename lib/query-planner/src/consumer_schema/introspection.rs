use std::collections::HashMap;

use crate::{
    ast::{
        operation::OperationDefinition, selection_item::SelectionItem,
        selection_set::InlineFragmentSelection,
    },
    consumer_schema::value_from_ast::value_from_ast,
};
use graphql_parser::schema::TypeDefinition;
use graphql_tools::introspection::{
    IntrospectionDirective, IntrospectionEnumType, IntrospectionEnumValue, IntrospectionField,
    IntrospectionInputObjectType, IntrospectionInputTypeRef, IntrospectionInputValue,
    IntrospectionInterfaceType, IntrospectionNamedTypeRef, IntrospectionObjectType,
    IntrospectionOutputTypeRef, IntrospectionQuery, IntrospectionScalarType, IntrospectionSchema,
    IntrospectionType, IntrospectionUnionType,
};

fn introspection_output_type_ref_from_ast(
    ast: &graphql_parser::schema::Type<'static, String>,
    type_ast_map: &HashMap<String, graphql_parser::schema::Definition<'static, String>>,
) -> graphql_tools::introspection::IntrospectionOutputTypeRef {
    match ast {
        graphql_parser::schema::Type::ListType(of_type) => IntrospectionOutputTypeRef::LIST {
            of_type: Some(Box::new(introspection_output_type_ref_from_ast(
                of_type,
                type_ast_map,
            ))),
        },
        graphql_parser::schema::Type::NonNullType(of_type) => {
            IntrospectionOutputTypeRef::NON_NULL {
                of_type: Some(Box::new(introspection_output_type_ref_from_ast(
                    of_type,
                    type_ast_map,
                ))),
            }
        }
        graphql_parser::schema::Type::NamedType(named_type) => {
            let named_type_definition = type_ast_map
                .get(named_type)
                .unwrap_or_else(|| panic!("Type {} not found in type AST map", named_type));
            match named_type_definition {
                graphql_parser::schema::Definition::TypeDefinition(TypeDefinition::Scalar(
                    scalar_type,
                )) => IntrospectionOutputTypeRef::SCALAR(IntrospectionNamedTypeRef {
                    name: scalar_type.name.to_string(),
                }),
                graphql_parser::schema::Definition::TypeDefinition(TypeDefinition::Object(
                    object_type,
                )) => IntrospectionOutputTypeRef::OBJECT(IntrospectionNamedTypeRef {
                    name: object_type.name.to_string(),
                }),
                graphql_parser::schema::Definition::TypeDefinition(TypeDefinition::Interface(
                    interface_type,
                )) => IntrospectionOutputTypeRef::INTERFACE(IntrospectionNamedTypeRef {
                    name: interface_type.name.to_string(),
                }),
                graphql_parser::schema::Definition::TypeDefinition(TypeDefinition::Union(
                    union_type,
                )) => IntrospectionOutputTypeRef::UNION(IntrospectionNamedTypeRef {
                    name: union_type.name.to_string(),
                }),
                graphql_parser::schema::Definition::TypeDefinition(TypeDefinition::Enum(
                    enum_type,
                )) => IntrospectionOutputTypeRef::ENUM(IntrospectionNamedTypeRef {
                    name: enum_type.name.to_string(),
                }),
                graphql_parser::schema::Definition::TypeDefinition(
                    TypeDefinition::InputObject(input_object_type),
                ) => IntrospectionOutputTypeRef::INPUT_OBJECT(IntrospectionNamedTypeRef {
                    name: input_object_type.name.to_string(),
                }),
                _ => panic!("Unsupported type definition for introspection"),
            }
        }
    }
}

fn introspection_input_type_ref_from_ast(
    ast: &graphql_parser::schema::Type<'static, String>,
    type_ast_map: &HashMap<String, graphql_parser::schema::Definition<'static, String>>,
) -> IntrospectionInputTypeRef {
    match ast {
        graphql_parser::schema::Type::ListType(of_type) => IntrospectionInputTypeRef::LIST {
            of_type: Some(Box::new(introspection_input_type_ref_from_ast(
                of_type,
                type_ast_map,
            ))),
        },
        graphql_parser::schema::Type::NonNullType(of_type) => IntrospectionInputTypeRef::NON_NULL {
            of_type: Some(Box::new(introspection_input_type_ref_from_ast(
                of_type,
                type_ast_map,
            ))),
        },
        graphql_parser::schema::Type::NamedType(named_type) => {
            let named_type_definition = type_ast_map
                .get(named_type)
                .unwrap_or_else(|| panic!("Type {} not found in type AST map", named_type));
            match named_type_definition {
                graphql_parser::schema::Definition::TypeDefinition(TypeDefinition::Scalar(
                    scalar_type,
                )) => IntrospectionInputTypeRef::SCALAR(IntrospectionNamedTypeRef {
                    name: scalar_type.name.to_string(),
                }),
                graphql_parser::schema::Definition::TypeDefinition(TypeDefinition::Enum(
                    enum_type,
                )) => IntrospectionInputTypeRef::ENUM(IntrospectionNamedTypeRef {
                    name: enum_type.name.to_string(),
                }),
                graphql_parser::schema::Definition::TypeDefinition(
                    TypeDefinition::InputObject(input_object_type),
                ) => IntrospectionInputTypeRef::INPUT_OBJECT(IntrospectionNamedTypeRef {
                    name: input_object_type.name.to_string(),
                }),
                _ => panic!("Unsupported type definition for introspection"),
            }
        }
    }
}

pub fn introspection_query_from_ast(
    ast: &graphql_parser::schema::Document<'static, String>,
) -> IntrospectionQuery {
    // Add known scalar types to the type AST map
    let mut type_ast_map: HashMap<String, graphql_parser::schema::Definition<'static, String>> =
        HashMap::new();
    let mut schema_definition: Option<&graphql_parser::schema::SchemaDefinition<'static, String>> =
        None;
    for definition in &ast.definitions {
        let type_name = match &definition {
            graphql_parser::schema::Definition::TypeDefinition(TypeDefinition::Scalar(
                scalar_type,
            )) => Some(&scalar_type.name),
            graphql_parser::schema::Definition::TypeDefinition(TypeDefinition::Object(
                object_type,
            )) => Some(&object_type.name),
            graphql_parser::schema::Definition::TypeDefinition(TypeDefinition::Interface(
                interface_type,
            )) => Some(&interface_type.name),
            graphql_parser::schema::Definition::TypeDefinition(TypeDefinition::Union(
                union_type,
            )) => Some(&union_type.name),
            graphql_parser::schema::Definition::TypeDefinition(TypeDefinition::Enum(enum_type)) => {
                Some(&enum_type.name)
            }
            graphql_parser::schema::Definition::TypeDefinition(TypeDefinition::InputObject(
                input_object_type,
            )) => Some(&input_object_type.name),
            graphql_parser::schema::Definition::DirectiveDefinition(directive) => {
                Some(&directive.name)
            }
            graphql_parser::schema::Definition::SchemaDefinition(schema_definition_ast) => {
                schema_definition = Some(schema_definition_ast);
                None
            }
            _ => None,
        };
        if let Some(type_name) = type_name {
            type_ast_map.insert(type_name.clone(), definition.clone());
        }
    }

    let mut types = vec![];
    let mut directives = vec![];

    for definition in type_ast_map.values() {
        match definition {
            graphql_parser::schema::Definition::TypeDefinition(TypeDefinition::Scalar(
                scalar_type,
            )) => {
                let builtin_props = get_builtin_props_from_directives(&scalar_type.directives);
                types.push(IntrospectionType::SCALAR(IntrospectionScalarType {
                    name: scalar_type.name.to_string(),
                    description: scalar_type.description.clone(),
                    specified_by_url: builtin_props.specified_by_url,
                }));
            }
            graphql_parser::schema::Definition::TypeDefinition(TypeDefinition::Object(
                object_type,
            )) => {
                if !object_type.name.starts_with("__") {
                    // Skip introspection types like __Schema, __Type, etc.
                    types.push(IntrospectionType::OBJECT(IntrospectionObjectType {
                        name: object_type.name.to_string(),
                        description: object_type.description.clone(),
                        fields: object_type
                            .fields
                            .iter()
                            .filter_map(|field| {
                                if field.name.starts_with("__") {
                                    // Skip introspection fields
                                    None
                                } else {
                                    let builtin_props =
                                        get_builtin_props_from_directives(&field.directives);
                                    Some(IntrospectionField {
                                        name: field.name.to_string(),
                                        description: field.description.clone(),
                                        is_deprecated: builtin_props.is_deprecated,
                                        deprecation_reason: builtin_props.deprecation_reason,
                                        args: field
                                            .arguments
                                            .iter()
                                            .map(|arg| {
                                                let builtin_props =
                                                    get_builtin_props_from_directives(
                                                        &arg.directives,
                                                    );
                                                IntrospectionInputValue {
                                                    name: arg.name.to_string(),
                                                    description: arg.description.clone(),
                                                    type_ref: Some(
                                                        introspection_input_type_ref_from_ast(
                                                            &arg.value_type,
                                                            &type_ast_map,
                                                        ),
                                                    ),
                                                    default_value: arg.default_value.as_ref().map(
                                                        |v| {
                                                            serde_json::Value::String(v.to_string())
                                                        },
                                                    ),
                                                    is_deprecated: builtin_props.is_deprecated,
                                                    deprecation_reason: builtin_props
                                                        .deprecation_reason,
                                                }
                                            })
                                            .collect(),
                                        type_ref: introspection_output_type_ref_from_ast(
                                            &field.field_type,
                                            &type_ast_map,
                                        ),
                                    })
                                }
                            })
                            .collect(),
                        interfaces: object_type
                            .implements_interfaces
                            .iter()
                            .map(|i| IntrospectionNamedTypeRef {
                                name: i.to_string(),
                            })
                            .collect(),
                    }));
                }
            }
            graphql_parser::schema::Definition::TypeDefinition(TypeDefinition::Interface(
                interface_type,
            )) => {
                types.push(IntrospectionType::INTERFACE(IntrospectionInterfaceType {
                    name: interface_type.name.to_string(),
                    description: interface_type.description.clone(),
                    fields: interface_type
                        .fields
                        .iter()
                        .map(|field| {
                            let builtin_props =
                                get_builtin_props_from_directives(&field.directives);
                            IntrospectionField {
                                name: field.name.to_string(),
                                description: field.description.clone(),
                                is_deprecated: builtin_props.is_deprecated,
                                deprecation_reason: builtin_props.deprecation_reason,
                                args: field
                                    .arguments
                                    .iter()
                                    .map(|arg| {
                                        let builtin_props =
                                            get_builtin_props_from_directives(&arg.directives);
                                        IntrospectionInputValue {
                                            name: arg.name.to_string(),
                                            description: arg.description.clone(),
                                            type_ref: Some(introspection_input_type_ref_from_ast(
                                                &arg.value_type,
                                                &type_ast_map,
                                            )),
                                            default_value: arg
                                                .default_value
                                                .as_ref()
                                                .map(|v| serde_json::Value::String(v.to_string())),
                                            is_deprecated: builtin_props.is_deprecated,
                                            deprecation_reason: builtin_props.deprecation_reason,
                                        }
                                    })
                                    .collect(),
                                type_ref: introspection_output_type_ref_from_ast(
                                    &field.field_type,
                                    &type_ast_map,
                                ),
                            }
                        })
                        .collect(),
                    interfaces: Some(
                        interface_type
                            .implements_interfaces
                            .iter()
                            .map(|i| IntrospectionNamedTypeRef {
                                name: i.to_string(),
                            })
                            .collect(),
                    ),
                    // TODO: Handle possible types
                    possible_types: vec![],
                }));
            }
            graphql_parser::schema::Definition::TypeDefinition(TypeDefinition::Union(
                union_type,
            )) => {
                types.push(IntrospectionType::UNION(IntrospectionUnionType {
                    name: union_type.name.to_string(),
                    description: union_type.description.clone(),
                    possible_types: union_type
                        .types
                        .iter()
                        .map(|t| IntrospectionNamedTypeRef {
                            name: t.to_string(),
                        })
                        .collect(),
                }));
            }
            graphql_parser::schema::Definition::TypeDefinition(TypeDefinition::Enum(enum_type)) => {
                types.push(IntrospectionType::ENUM(IntrospectionEnumType {
                    name: enum_type.name.to_string(),
                    description: enum_type.description.clone(),
                    enum_values: enum_type
                        .values
                        .iter()
                        .map(|enum_value| {
                            let builtin_props =
                                get_builtin_props_from_directives(&enum_value.directives);
                            IntrospectionEnumValue {
                                name: enum_value.name.to_string(),
                                description: enum_value.description.clone(),
                                is_deprecated: builtin_props.is_deprecated,
                                deprecation_reason: builtin_props.deprecation_reason,
                            }
                        })
                        .collect(),
                }));
            }
            graphql_parser::schema::Definition::TypeDefinition(TypeDefinition::InputObject(
                input_object_type,
            )) => {
                types.push(IntrospectionType::INPUT_OBJECT(
                    IntrospectionInputObjectType {
                        name: input_object_type.name.to_string(),
                        description: input_object_type.description.clone(),
                        input_fields: input_object_type
                            .fields
                            .iter()
                            .map(|field| {
                                let builtin_props =
                                    get_builtin_props_from_directives(&field.directives);
                                IntrospectionInputValue {
                                    name: field.name.to_string(),
                                    description: field.description.clone(),
                                    type_ref: Some(introspection_input_type_ref_from_ast(
                                        &field.value_type,
                                        &type_ast_map,
                                    )),
                                    default_value: field
                                        .default_value
                                        .as_ref()
                                        .map(|v| serde_json::Value::String(v.to_string())),
                                    is_deprecated: builtin_props.is_deprecated,
                                    deprecation_reason: builtin_props.deprecation_reason,
                                }
                            })
                            .collect(),
                    },
                ));
            }
            graphql_parser::schema::Definition::DirectiveDefinition(directive) => {
                directives.push(IntrospectionDirective {
                    name: directive.name.to_string(),
                    description: directive.description.clone(),
                    locations: directive
                        .locations
                        .iter()
                        .map(|l| {
                            match l {
                                graphql_parser::schema::DirectiveLocation::Query => graphql_tools::introspection::DirectiveLocation::QUERY,
                                graphql_parser::schema::DirectiveLocation::Mutation => graphql_tools::introspection::DirectiveLocation::MUTATION,
                                graphql_parser::schema::DirectiveLocation::Subscription => graphql_tools::introspection::DirectiveLocation::SUBSCRIPTION,
                                graphql_parser::schema::DirectiveLocation::Field => graphql_tools::introspection::DirectiveLocation::FIELD,
                                graphql_parser::schema::DirectiveLocation::FragmentDefinition => graphql_tools::introspection::DirectiveLocation::FRAGMENT_DEFINITION,
                                graphql_parser::schema::DirectiveLocation::FragmentSpread => graphql_tools::introspection::DirectiveLocation::FRAGMENT_SPREAD,
                                graphql_parser::schema::DirectiveLocation::InlineFragment => graphql_tools::introspection::DirectiveLocation::INLINE_FRAGMENT,
                                graphql_parser::schema::DirectiveLocation::VariableDefinition => graphql_tools::introspection::DirectiveLocation::VARIABLE_DEFINITION,
                                graphql_parser::schema::DirectiveLocation::Schema => graphql_tools::introspection::DirectiveLocation::SCHEMA,
                                graphql_parser::schema::DirectiveLocation::Scalar => graphql_tools::introspection::DirectiveLocation::SCALAR,
                                graphql_parser::schema::DirectiveLocation::Object => graphql_tools::introspection::DirectiveLocation::OBJECT,
                                graphql_parser::schema::DirectiveLocation::FieldDefinition => graphql_tools::introspection::DirectiveLocation::FIELD_DEFINITION,
                                graphql_parser::schema::DirectiveLocation::ArgumentDefinition => graphql_tools::introspection::DirectiveLocation::ARGUMENT_DEFINITION,
                                graphql_parser::schema::DirectiveLocation::Interface => graphql_tools::introspection::DirectiveLocation::INTERFACE,
                                graphql_parser::schema::DirectiveLocation::Union => graphql_tools::introspection::DirectiveLocation::UNION,
                                graphql_parser::schema::DirectiveLocation::Enum => graphql_tools::introspection::DirectiveLocation::ENUM,
                                graphql_parser::schema::DirectiveLocation::EnumValue => graphql_tools::introspection::DirectiveLocation::ENUM_VALUE,
                                graphql_parser::schema::DirectiveLocation::InputObject => graphql_tools::introspection::DirectiveLocation::INPUT_OBJECT,
                                graphql_parser::schema::DirectiveLocation::InputFieldDefinition => graphql_tools::introspection::DirectiveLocation::INPUT_FIELD_DEFINITION,
                            }
                        })
                        .collect(),
                    is_repeatable: Some(directive.repeatable),
                    args: directive
                        .arguments
                        .iter()
                        .map(|arg: &graphql_parser::schema::InputValue<'_, String>| {
                            let builtin_props =
                                get_builtin_props_from_directives(&arg.directives);
                            IntrospectionInputValue {
                                name: arg.name.to_string(),
                                description: arg.description.clone(),
                                type_ref: Some(introspection_input_type_ref_from_ast(&arg.value_type, &type_ast_map)),
                                default_value: arg
                                    .default_value
                                    .as_ref()
                                    .map(|v| serde_json::Value::String(v.to_string())),
                                is_deprecated: builtin_props.is_deprecated,
                                deprecation_reason: builtin_props.deprecation_reason,
                            }
                        })
                        .collect(),
                });
            }
            _ => {
                // Ignore other definitions like TypeExtension, SchemaDefinition, etc.
            }
        }
    }

    IntrospectionQuery {
        __schema: IntrospectionSchema {
            query_type: IntrospectionNamedTypeRef {
                name: schema_definition
                    .as_ref()
                    .and_then(|sd| sd.query.as_ref())
                    .map_or("Query".to_string(), |qt| qt.to_string()),
            },
            mutation_type: schema_definition
                .as_ref()
                .and_then(|sd| sd.mutation.as_ref())
                .map(|mt| IntrospectionNamedTypeRef {
                    name: mt.to_string(),
                }),
            subscription_type: schema_definition
                .as_ref()
                .and_then(|sd| sd.subscription.as_ref())
                .map(|st| IntrospectionNamedTypeRef {
                    name: st.to_string(),
                }),
            types,
            directives,
            // TODO: Description missing on graphql_parser::schema::SchemaDefinition
            description: None,
        },
    }
}

struct BuiltinDirectiveProps {
    is_deprecated: Option<bool>,
    deprecation_reason: Option<String>,
    one_of: Option<bool>,
    specified_by_url: Option<String>,
}

fn get_builtin_props_from_directives(
    directives: &[graphql_parser::schema::Directive<'static, String>],
) -> BuiltinDirectiveProps {
    let mut props = BuiltinDirectiveProps {
        is_deprecated: None,
        deprecation_reason: None,
        one_of: None,
        specified_by_url: None,
    };
    for directive in directives {
        match directive.name.as_str() {
            "deprecated" => {
                props.is_deprecated = Some(true);
                for (arg_name, arg_value) in &directive.arguments {
                    if arg_name == "reason" {
                        props.deprecation_reason = Some(
                            value_from_ast(arg_value, &None)
                                .expect("Unable to parse deprecated")
                                .to_string(),
                        );
                    }
                }
            }
            "oneOf" => {
                props.one_of = Some(true);
            }
            "specifiedBy" => {
                for (arg_name, arg_value) in &directive.arguments {
                    if arg_name == "url" {
                        props.specified_by_url = Some(
                            value_from_ast(arg_value, &None)
                                .expect("Unable to parse specifiedBy URL")
                                .to_string(),
                        );
                    }
                }
            }
            _ => {}
        }
    }
    props
}

fn filter_introspection_selections(
    selection_set: &crate::ast::selection_set::SelectionSet,
) -> (bool, crate::ast::selection_set::SelectionSet) {
    let mut has_introspection = false;
    let filtered_selections: Vec<SelectionItem> = selection_set
        .items
        .iter()
        .filter_map(|item| {
            match item {
                SelectionItem::Field(field) => {
                    if field.name.starts_with("__") {
                        has_introspection = true;
                        None // Skip introspection fields except __typename
                    } else {
                        Some(item.clone())
                    }
                }
                SelectionItem::InlineFragment(inline_fragment) => {
                    let (has_introspection_in_fragment, filtered_selection_set) =
                        filter_introspection_selections(&inline_fragment.selections);
                    if has_introspection_in_fragment {
                        has_introspection = true;
                        Some(SelectionItem::InlineFragment(InlineFragmentSelection {
                            type_condition: inline_fragment.type_condition.clone(),
                            selections: filtered_selection_set,
                        }))
                    } else {
                        Some(item.clone())
                    }
                }
            }
        })
        .collect();
    (
        has_introspection,
        crate::ast::selection_set::SelectionSet {
            items: filtered_selections,
        },
    )
}

pub fn filter_introspection_fields_in_operation(
    operation: &OperationDefinition,
) -> (bool, OperationDefinition) {
    let (has_introspection, filtered_selection_set) =
        filter_introspection_selections(&operation.selection_set);

    (
        has_introspection,
        OperationDefinition {
            name: operation.name.clone(),
            operation_kind: operation.operation_kind.clone(),
            selection_set: filtered_selection_set,
            variable_definitions: operation.variable_definitions.clone(),
        },
    )
}
