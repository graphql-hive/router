use std::collections::HashMap;

use lazy_static::lazy_static;

use crate::static_graphql::query::{
    self, Directive, FragmentSpread, OperationDefinition, SelectionSet, Type, Value,
    VariableDefinition,
};
use crate::static_graphql::schema::{
    self, DirectiveDefinition, InputValue, InterfaceType, ObjectType, TypeDefinition, UnionType,
};

pub trait FieldByNameExtension {
    fn field_by_name(&self, name: &str) -> Option<&schema::Field>;
    fn input_field_by_name(&self, name: &str) -> Option<&InputValue>;
}

impl FieldByNameExtension for TypeDefinition {
    fn field_by_name(&self, name: &str) -> Option<&schema::Field> {
        match self {
            TypeDefinition::Object(object) => {
                object.fields.iter().find(|field| field.name.eq(name))
            }
            TypeDefinition::Interface(interface) => {
                interface.fields.iter().find(|field| field.name.eq(name))
            }
            _ => None,
        }
    }

    fn input_field_by_name(&self, name: &str) -> Option<&InputValue> {
        match self {
            TypeDefinition::InputObject(input_object) => {
                input_object.fields.iter().find(|field| field.name.eq(name))
            }
            _ => None,
        }
    }
}

pub trait OperationDefinitionExtension {
    fn variable_definitions(&self) -> &[VariableDefinition];
    fn directives(&self) -> &[Directive];
    fn selection_set(&self) -> &SelectionSet;
}

impl OperationDefinitionExtension for OperationDefinition {
    fn variable_definitions(&self) -> &[VariableDefinition] {
        match self {
            OperationDefinition::Query(query) => &query.variable_definitions,
            OperationDefinition::SelectionSet(_) => &[],
            OperationDefinition::Mutation(mutation) => &mutation.variable_definitions,
            OperationDefinition::Subscription(subscription) => &subscription.variable_definitions,
        }
    }

    fn selection_set(&self) -> &SelectionSet {
        match self {
            OperationDefinition::Query(query) => &query.selection_set,
            OperationDefinition::SelectionSet(selection_set) => selection_set,
            OperationDefinition::Mutation(mutation) => &mutation.selection_set,
            OperationDefinition::Subscription(subscription) => &subscription.selection_set,
        }
    }

    fn directives(&self) -> &[Directive] {
        match self {
            OperationDefinition::Query(query) => &query.directives,
            OperationDefinition::SelectionSet(_) => &[],
            OperationDefinition::Mutation(mutation) => &mutation.directives,
            OperationDefinition::Subscription(subscription) => &subscription.directives,
        }
    }
}

pub trait SchemaDocumentExtension {
    fn type_by_name(&self, name: &str) -> Option<&TypeDefinition>;
    fn type_map(&self) -> HashMap<&str, &TypeDefinition>;
    fn directive_by_name(&self, name: &str) -> Option<&DirectiveDefinition>;
    fn object_type_by_name(&self, name: &str) -> Option<&ObjectType>;
    fn schema_definition(&self) -> &schema::SchemaDefinition;
    fn query_type(&self) -> &ObjectType;
    fn mutation_type(&self) -> Option<&ObjectType>;
    fn subscription_type(&self) -> Option<&ObjectType>;
    fn is_subtype(&self, sub_type: &Type, super_type: &Type) -> bool;
    fn is_named_subtype(&self, sub_type_name: &str, super_type_name: &str) -> bool;
    fn is_possible_type(
        &self,
        abstract_type: &TypeDefinition,
        possible_type: &TypeDefinition,
    ) -> bool;
}

impl SchemaDocumentExtension for schema::Document {
    fn type_by_name(&self, name: &str) -> Option<&TypeDefinition> {
        for def in &self.definitions {
            if let schema::Definition::TypeDefinition(type_def) = def {
                if type_def.name().eq(name) {
                    return Some(type_def);
                }
            }
        }

        None
    }

    fn directive_by_name(&self, name: &str) -> Option<&DirectiveDefinition> {
        for def in &self.definitions {
            if let schema::Definition::DirectiveDefinition(directive_def) = def {
                if directive_def.name.eq(name) {
                    return Some(directive_def);
                }
            }
        }

        None
    }

    fn schema_definition(&self) -> &schema::SchemaDefinition {
        lazy_static! {
            static ref DEFAULT_SCHEMA_DEF: schema::SchemaDefinition = {
                schema::SchemaDefinition {
                    query: Some("Query".to_string()),
                    ..Default::default()
                }
            };
        }
        self.definitions
            .iter()
            .find_map(|definition| match definition {
                schema::Definition::SchemaDefinition(schema_definition) => Some(schema_definition),
                _ => None,
            })
            .unwrap_or(&*DEFAULT_SCHEMA_DEF)
    }

    fn query_type(&self) -> &ObjectType {
        lazy_static! {
            static ref QUERY: String = "Query".to_string();
        }

        let schema_definition = self.schema_definition();

        self.object_type_by_name(schema_definition.query.as_ref().unwrap_or(&QUERY))
            .unwrap()
    }

    fn mutation_type(&self) -> Option<&ObjectType> {
        self.schema_definition()
            .mutation
            .as_ref()
            .and_then(|name| self.object_type_by_name(name))
    }

    fn subscription_type(&self) -> Option<&ObjectType> {
        self.schema_definition()
            .subscription
            .as_ref()
            .and_then(|name| self.object_type_by_name(name))
    }

    fn object_type_by_name(&self, name: &str) -> Option<&ObjectType> {
        match self.type_by_name(name) {
            Some(TypeDefinition::Object(object_def)) => Some(object_def),
            _ => None,
        }
    }

    fn type_map(&self) -> HashMap<&str, &TypeDefinition> {
        let mut type_map = HashMap::new();

        for def in &self.definitions {
            if let schema::Definition::TypeDefinition(type_def) = def {
                type_map.insert(type_def.name(), type_def);
            }
        }

        type_map
    }

    fn is_named_subtype(&self, sub_type_name: &str, super_type_name: &str) -> bool {
        if sub_type_name == super_type_name {
            true
        } else if let (Some(sub_type), Some(super_type)) = (
            self.type_by_name(sub_type_name),
            self.type_by_name(super_type_name),
        ) {
            super_type.is_abstract_type() && self.is_possible_type(super_type, sub_type)
        } else {
            false
        }
    }

    fn is_possible_type(
        &self,
        abstract_type: &TypeDefinition,
        possible_type: &TypeDefinition,
    ) -> bool {
        match abstract_type {
            TypeDefinition::Union(union_typedef) => union_typedef
                .types
                .iter()
                .any(|t| t == possible_type.name()),
            TypeDefinition::Interface(interface_typedef) => {
                let implementes_interfaces = possible_type.interfaces();

                implementes_interfaces.contains(&interface_typedef.name)
            }
            _ => false,
        }
    }

    fn is_subtype(&self, sub_type: &Type, super_type: &Type) -> bool {
        // Equivalent type is a valid subtype
        if sub_type == super_type {
            return true;
        }

        // If superType is non-null, maybeSubType must also be non-null.
        if super_type.is_non_null() {
            if sub_type.is_non_null() {
                return self.is_subtype(sub_type.of_type(), super_type.of_type());
            }
            return false;
        }

        if sub_type.is_non_null() {
            // If superType is nullable, maybeSubType may be non-null or nullable.
            return self.is_subtype(sub_type.of_type(), super_type);
        }

        // If superType type is a list, maybeSubType type must also be a list.
        if super_type.is_list_type() {
            if sub_type.is_list_type() {
                return self.is_subtype(sub_type.of_type(), super_type.of_type());
            }

            return false;
        }

        if sub_type.is_list_type() {
            // If superType is nullable, maybeSubType may be non-null or nullable.
            return false;
        }

        // If superType type is an abstract type, check if it is super type of maybeSubType.
        // Otherwise, the child type is not a valid subtype of the parent type.
        if let (Some(sub_type), Some(super_type)) = (
            self.type_by_name(sub_type.inner_type()),
            self.type_by_name(super_type.inner_type()),
        ) {
            return super_type.is_abstract_type()
                && (sub_type.is_interface_type() || sub_type.is_object_type())
                && self.is_possible_type(super_type, sub_type);
        }

        false
    }
}

pub trait TypeExtension {
    fn inner_type(&self) -> &str;
    fn is_non_null(&self) -> bool;
    fn is_list_type(&self) -> bool;
    fn is_named_type(&self) -> bool;
    fn of_type(&self) -> &Type;
}

impl TypeExtension for Type {
    fn inner_type(&self) -> &str {
        match self {
            Type::NamedType(name) => name.as_str(),
            Type::ListType(child) => child.inner_type(),
            Type::NonNullType(child) => child.inner_type(),
        }
    }

    fn of_type(&self) -> &Type {
        match self {
            Type::ListType(child) => child,
            Type::NonNullType(child) => child,
            Type::NamedType(_) => self,
        }
    }

    fn is_non_null(&self) -> bool {
        matches!(self, Type::NonNullType(_))
    }

    fn is_list_type(&self) -> bool {
        matches!(self, Type::ListType(_))
    }

    fn is_named_type(&self) -> bool {
        matches!(self, Type::NamedType(_))
    }
}

pub trait ValueExtension {
    fn compare(&self, other: &Self) -> bool;
    fn variables_in_use(&self) -> Vec<&str>;
}

impl ValueExtension for Value {
    fn compare(&self, other: &Self) -> bool {
        match (self, other) {
            (Value::Null, Value::Null) => true,
            (Value::Boolean(a), Value::Boolean(b)) => a == b,
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Float(a), Value::Float(b)) => a == b,
            (Value::String(a), Value::String(b)) => a.eq(b),
            (Value::Enum(a), Value::Enum(b)) => a.eq(b),
            (Value::List(a), Value::List(b)) => a.iter().zip(b.iter()).all(|(a, b)| a.compare(b)),
            (Value::Object(a), Value::Object(b)) => {
                a.iter().zip(b.iter()).all(|(a, b)| a.1.compare(b.1))
            }
            (Value::Variable(a), Value::Variable(b)) => a.eq(b),
            _ => false,
        }
    }

    fn variables_in_use(&self) -> Vec<&str> {
        match self {
            Value::Variable(v) => vec![v],
            Value::List(list) => list.iter().flat_map(|v| v.variables_in_use()).collect(),
            Value::Object(object) => object
                .iter()
                .flat_map(|(_, v)| v.variables_in_use())
                .collect(),
            _ => vec![],
        }
    }
}

pub trait InputValueHelpers {
    fn is_required(&self) -> bool;
}

impl InputValueHelpers for InputValue {
    fn is_required(&self) -> bool {
        if let Type::NonNullType(_inner_type) = &self.value_type {
            if self.default_value.is_none() {
                return true;
            }
        }

        false
    }
}

pub trait AbstractTypeDefinitionExtension {
    fn is_implemented_by(&self, other_type: &dyn ImplementingInterfaceExtension) -> bool;
}

pub trait TypeDefinitionExtension {
    fn is_leaf_type(&self) -> bool;
    fn is_composite_type(&self) -> bool;
    fn is_input_type(&self) -> bool;
    fn is_object_type(&self) -> bool;
    fn is_union_type(&self) -> bool;
    fn is_interface_type(&self) -> bool;
    fn is_enum_type(&self) -> bool;
    fn is_scalar_type(&self) -> bool;
    fn is_abstract_type(&self) -> bool;
    fn name(&self) -> &str;
}

pub trait ImplementingInterfaceExtension {
    fn interfaces(&self) -> Vec<String>;
    fn has_sub_type(&self, other_type: &TypeDefinition) -> bool;
    fn has_concrete_sub_type(&self, concrete_type: &ObjectType) -> bool;
}

impl ImplementingInterfaceExtension for TypeDefinition {
    fn interfaces(&self) -> Vec<String> {
        match self {
            schema::TypeDefinition::Object(o) => o.interfaces(),
            schema::TypeDefinition::Interface(i) => i.interfaces(),
            _ => vec![],
        }
    }

    fn has_sub_type(&self, other_type: &TypeDefinition) -> bool {
        match self {
            TypeDefinition::Interface(interface_type) => {
                interface_type.is_implemented_by(other_type)
            }
            TypeDefinition::Union(union_type) => union_type.has_sub_type(other_type.name()),
            _ => false,
        }
    }

    fn has_concrete_sub_type(&self, concrete_type: &ObjectType) -> bool {
        match self {
            TypeDefinition::Interface(interface_type) => {
                interface_type.is_implemented_by(concrete_type)
            }
            TypeDefinition::Union(union_type) => union_type.has_sub_type(&concrete_type.name),
            _ => false,
        }
    }
}

pub trait PossibleTypesExtension {
    fn possible_types<'a>(&self, schema: &'a schema::Document) -> Vec<&'a ObjectType>;
}

impl PossibleTypesExtension for TypeDefinition {
    fn possible_types<'a>(&self, schema: &'a schema::Document) -> Vec<&'a ObjectType> {
        match self {
            TypeDefinition::Object(_) => vec![],
            TypeDefinition::InputObject(_) => vec![],
            TypeDefinition::Enum(_) => vec![],
            TypeDefinition::Scalar(_) => vec![],
            TypeDefinition::Interface(i) => schema
                .type_map()
                .iter()
                .filter_map(|(_type_name, type_def)| {
                    if let TypeDefinition::Object(o) = type_def {
                        if i.is_implemented_by(*type_def) {
                            return Some(o);
                        }
                    }

                    None
                })
                .collect(),
            TypeDefinition::Union(u) => u
                .types
                .iter()
                .filter_map(|type_name| {
                    if let Some(TypeDefinition::Object(o)) = schema.type_by_name(type_name) {
                        return Some(o);
                    }

                    None
                })
                .collect(),
        }
    }
}

impl ImplementingInterfaceExtension for InterfaceType {
    fn interfaces(&self) -> Vec<String> {
        self.implements_interfaces.clone()
    }

    fn has_sub_type(&self, other_type: &TypeDefinition) -> bool {
        self.is_implemented_by(other_type)
    }

    fn has_concrete_sub_type(&self, concrete_type: &ObjectType) -> bool {
        self.is_implemented_by(concrete_type)
    }
}

impl ImplementingInterfaceExtension for ObjectType {
    fn interfaces(&self) -> Vec<String> {
        self.implements_interfaces.clone()
    }

    fn has_sub_type(&self, _other_type: &TypeDefinition) -> bool {
        false
    }

    fn has_concrete_sub_type(&self, _concrete_type: &ObjectType) -> bool {
        false
    }
}

pub trait SubTypeExtension {
    fn has_sub_type(&self, other_type_name: &str) -> bool;
}

impl SubTypeExtension for UnionType {
    fn has_sub_type(&self, other_type_name: &str) -> bool {
        self.types.iter().any(|v| other_type_name.eq(v))
    }
}

impl AbstractTypeDefinitionExtension for InterfaceType {
    fn is_implemented_by(&self, other_type: &dyn ImplementingInterfaceExtension) -> bool {
        other_type.interfaces().iter().any(|v| self.name.eq(v))
    }
}

impl TypeDefinitionExtension for Option<&schema::TypeDefinition> {
    fn is_leaf_type(&self) -> bool {
        match self {
            Some(t) => t.is_leaf_type(),
            _ => false,
        }
    }

    fn is_composite_type(&self) -> bool {
        match self {
            Some(t) => t.is_composite_type(),
            _ => false,
        }
    }

    fn is_input_type(&self) -> bool {
        match self {
            Some(t) => t.is_input_type(),
            _ => false,
        }
    }

    fn is_interface_type(&self) -> bool {
        match self {
            Some(t) => t.is_interface_type(),
            _ => false,
        }
    }

    fn is_object_type(&self) -> bool {
        match self {
            Some(t) => t.is_object_type(),
            _ => false,
        }
    }

    fn is_union_type(&self) -> bool {
        match self {
            Some(t) => t.is_union_type(),
            _ => false,
        }
    }

    fn is_enum_type(&self) -> bool {
        match self {
            Some(t) => t.is_enum_type(),
            _ => false,
        }
    }

    fn is_scalar_type(&self) -> bool {
        match self {
            Some(t) => t.is_scalar_type(),
            _ => false,
        }
    }

    fn is_abstract_type(&self) -> bool {
        match self {
            Some(t) => t.is_abstract_type(),
            _ => false,
        }
    }

    fn name(&self) -> &str {
        match self {
            Some(t) => t.name(),
            _ => "",
        }
    }
}

impl TypeDefinitionExtension for schema::TypeDefinition {
    fn name(&self) -> &str {
        match self {
            schema::TypeDefinition::Object(o) => &o.name,
            schema::TypeDefinition::Interface(i) => &i.name,
            schema::TypeDefinition::Union(u) => &u.name,
            schema::TypeDefinition::Scalar(s) => &s.name,
            schema::TypeDefinition::Enum(e) => &e.name,
            schema::TypeDefinition::InputObject(i) => &i.name,
        }
    }

    fn is_abstract_type(&self) -> bool {
        matches!(
            self,
            schema::TypeDefinition::Interface(_) | schema::TypeDefinition::Union(_)
        )
    }

    fn is_interface_type(&self) -> bool {
        matches!(self, schema::TypeDefinition::Interface(_))
    }

    fn is_leaf_type(&self) -> bool {
        matches!(
            self,
            schema::TypeDefinition::Scalar(_) | schema::TypeDefinition::Enum(_)
        )
    }

    fn is_input_type(&self) -> bool {
        matches!(
            self,
            schema::TypeDefinition::Scalar(_)
                | schema::TypeDefinition::Enum(_)
                | schema::TypeDefinition::InputObject(_)
        )
    }

    fn is_composite_type(&self) -> bool {
        matches!(
            self,
            schema::TypeDefinition::Object(_)
                | schema::TypeDefinition::Interface(_)
                | schema::TypeDefinition::Union(_)
        )
    }

    fn is_object_type(&self) -> bool {
        matches!(self, schema::TypeDefinition::Object(_o))
    }

    fn is_union_type(&self) -> bool {
        matches!(self, schema::TypeDefinition::Union(_o))
    }

    fn is_enum_type(&self) -> bool {
        matches!(self, schema::TypeDefinition::Enum(_o))
    }

    fn is_scalar_type(&self) -> bool {
        matches!(self, schema::TypeDefinition::Scalar(_o))
    }
}

pub trait AstNodeWithName {
    fn node_name(&self) -> Option<&str>;
}

impl AstNodeWithName for query::OperationDefinition {
    fn node_name(&self) -> Option<&str> {
        match self {
            query::OperationDefinition::Query(q) => q.name.as_deref(),
            query::OperationDefinition::SelectionSet(_s) => None,
            query::OperationDefinition::Mutation(m) => m.name.as_deref(),
            query::OperationDefinition::Subscription(s) => s.name.as_deref(),
        }
    }
}

impl AstNodeWithName for query::FragmentDefinition {
    fn node_name(&self) -> Option<&str> {
        Some(&self.name)
    }
}

impl AstNodeWithName for query::FragmentSpread {
    fn node_name(&self) -> Option<&str> {
        Some(&self.fragment_name)
    }
}

pub trait FragmentSpreadExtraction {
    fn get_recursive_fragment_spreads(&self) -> Vec<&FragmentSpread>;
    fn get_fragment_spreads(&self) -> Vec<&FragmentSpread>;
}

impl FragmentSpreadExtraction for query::SelectionSet {
    fn get_recursive_fragment_spreads(&self) -> Vec<&FragmentSpread> {
        self.items
            .iter()
            .flat_map(|v| match v {
                query::Selection::FragmentSpread(f) => vec![f],
                query::Selection::Field(f) => f.selection_set.get_fragment_spreads(),
                query::Selection::InlineFragment(f) => f.selection_set.get_fragment_spreads(),
            })
            .collect()
    }

    fn get_fragment_spreads(&self) -> Vec<&FragmentSpread> {
        self.items
            .iter()
            .flat_map(|v| match v {
                query::Selection::FragmentSpread(f) => vec![f],
                _ => vec![],
            })
            .collect()
    }
}
