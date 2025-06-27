use std::collections::{HashMap, HashSet};

use graphql_parser::{
    query::Type,
    schema::{Definition, TypeDefinition},
};
use query_planner::consumer_schema::ConsumerSchema;
use serde_json::{json, Value};

#[derive(Debug)]
pub struct SchemaMetadata {
    pub possible_types: HashMap<String, HashSet<String>>,
    pub enum_values: HashMap<String, Vec<String>>,
    pub type_fields: HashMap<String, HashMap<String, String>>,
    pub introspection_schema_root_json: Value,
}

pub trait SchemaWithMetadata {
    fn schema_metadata(&self) -> SchemaMetadata;
}

impl SchemaWithMetadata for ConsumerSchema {
    fn schema_metadata(&self) -> SchemaMetadata {
        let mut first_possible_types: HashMap<String, Vec<String>> = HashMap::new();
        let mut type_fields: HashMap<String, HashMap<String, String>> = HashMap::new();
        let mut enum_values: HashMap<String, Vec<String>> = HashMap::new();

        for definition in &self.document.definitions {
            match definition {
                Definition::TypeDefinition(TypeDefinition::Enum(enum_type)) => {
                    let name = enum_type.name.to_string();
                    let mut values = vec![];
                    for enum_value in &enum_type.values {
                        values.push(enum_value.name.to_string());
                    }
                    enum_values.insert(name, values);
                }
                Definition::TypeDefinition(TypeDefinition::Object(object_type)) => {
                    let name = object_type.name.to_string();
                    let fields = type_fields.entry(name).or_default();
                    for field in &object_type.fields {
                        let field_type_name = field.field_type.type_name();
                        fields.insert(field.name.to_string(), field_type_name);
                    }

                    for interface in &object_type.implements_interfaces {
                        let interface_name = interface.to_string();
                        let possible_types_entry =
                            first_possible_types.entry(interface_name).or_default();
                        possible_types_entry.push(object_type.name.to_string());
                    }
                }
                Definition::TypeDefinition(TypeDefinition::Interface(interface_type)) => {
                    let name = interface_type.name.to_string();
                    let mut fields = HashMap::new();
                    for field in &interface_type.fields {
                        let field_type_name = field.field_type.type_name();
                        fields.insert(field.name.to_string(), field_type_name);
                    }
                    type_fields.insert(name, fields);
                    for interface_name in &interface_type.implements_interfaces {
                        let interface_name = interface_name.to_string();
                        let possible_types_entry =
                            first_possible_types.entry(interface_name).or_default();
                        possible_types_entry.push(interface_type.name.to_string());
                    }
                }
                Definition::TypeDefinition(TypeDefinition::Union(union_type)) => {
                    let name = union_type.name.to_string();
                    let mut types = vec![];
                    for member in &union_type.types {
                        types.push(member.to_string());
                    }
                    first_possible_types.insert(name, types);
                }
                _ => {}
            }
        }

        let mut final_possible_types: HashMap<String, HashSet<String>> = HashMap::new();
        // Re-iterate over the possible_types
        for (definition_name_of_x, first_possible_types_of_x) in &first_possible_types {
            let mut possible_types_of_x: HashSet<String> = HashSet::new();
            for definition_name_of_y in first_possible_types_of_x {
                possible_types_of_x.insert(definition_name_of_y.to_string());
                let possible_types_of_y = first_possible_types.get(definition_name_of_y);
                if let Some(possible_types_of_y) = possible_types_of_y {
                    for definition_name_of_z in possible_types_of_y {
                        possible_types_of_x.insert(definition_name_of_z.to_string());
                    }
                }
            }
            final_possible_types.insert(definition_name_of_x.to_string(), possible_types_of_x);
        }

        let introspection_query =
            crate::introspection::introspection_query_from_ast(&self.document);
        let introspection_schema_root_json = json!(introspection_query.__schema);

        SchemaMetadata {
            possible_types: final_possible_types,
            enum_values,
            type_fields,
            introspection_schema_root_json,
        }
    }
}

trait TypeName {
    fn type_name(&self) -> String;
}

impl TypeName for Type<'_, String> {
    fn type_name(&self) -> String {
        match self {
            graphql_parser::schema::Type::NamedType(named_type) => named_type.to_string(),
            graphql_parser::schema::Type::NonNullType(non_null_type) => non_null_type.type_name(),
            graphql_parser::schema::Type::ListType(list_type) => list_type.type_name(),
        }
    }
}
