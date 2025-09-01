use std::collections::{HashMap, HashSet};

use graphql_parser::{
    query::Type,
    schema::{Definition, TypeDefinition},
};
use hive_router_query_planner::consumer_schema::ConsumerSchema;

#[derive(Debug, Default)]
pub struct SchemaMetadata {
    pub possible_types: PossibleTypes,
    pub enum_values: HashMap<String, HashSet<String>>,
    pub type_fields: HashMap<String, HashMap<String, String>>,
    pub object_types: HashSet<String>,
    pub scalar_types: HashSet<String>,
}

impl SchemaMetadata {
    pub fn is_object_type(&self, name: &str) -> bool {
        self.object_types.contains(name)
    }

    pub fn is_scalar_type(&self, name: &str) -> bool {
        self.scalar_types.contains(name)
    }
}

#[derive(Debug, Default)]
pub struct PossibleTypes {
    map: HashMap<String, HashSet<String>>,
}

impl PossibleTypes {
    pub fn entity_satisfies_type_condition(&self, type_name: &str, type_condition: &str) -> bool {
        if type_name == type_condition {
            true
        } else if let Some(possible_types_of_type) = self.map.get(type_condition) {
            possible_types_of_type.contains(type_name)
        } else {
            false
        }
    }
    pub fn get_possible_types(&self, type_name: &str) -> HashSet<String> {
        let mut possible_types = self.map.get(type_name).cloned().unwrap_or_default();
        possible_types.insert(type_name.to_string());
        possible_types
    }
}

pub trait SchemaWithMetadata {
    fn schema_metadata(&self) -> SchemaMetadata;
}

impl SchemaWithMetadata for ConsumerSchema {
    fn schema_metadata(&self) -> SchemaMetadata {
        let mut first_possible_types: HashMap<String, Vec<String>> = HashMap::new();
        let mut type_fields: HashMap<String, HashMap<String, String>> = HashMap::new();
        let mut enum_values: HashMap<String, HashSet<String>> = HashMap::new();
        let mut scalar_types: HashSet<String> = HashSet::from_iter(vec![
            "Boolean".to_string(),
            "Float".to_string(),
            "Int".to_string(),
            "ID".to_string(),
            "String".to_string(),
        ]);
        let mut object_types: HashSet<String> = HashSet::new();

        for definition in &self.document.definitions {
            match definition {
                Definition::TypeDefinition(TypeDefinition::Enum(enum_type)) => {
                    let name = enum_type.name.to_string();
                    let mut values = HashSet::new();
                    for enum_value in &enum_type.values {
                        values.insert(enum_value.name.to_string());
                    }
                    enum_values.insert(name, values);
                }
                Definition::TypeDefinition(TypeDefinition::Object(object_type)) => {
                    let name = object_type.name.to_string();
                    object_types.insert(name.clone());
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
                Definition::TypeDefinition(TypeDefinition::Scalar(scalar_type)) => {
                    scalar_types.insert(scalar_type.name.to_string());
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

        SchemaMetadata {
            possible_types: PossibleTypes {
                map: final_possible_types,
            },
            enum_values,
            type_fields,
            object_types,
            scalar_types,
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
