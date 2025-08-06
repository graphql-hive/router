use std::collections::{HashMap, HashSet};

use graphql_parser::{
    query::Type,
    schema::{Definition, TypeDefinition},
};
use query_planner::consumer_schema::ConsumerSchema;

#[derive(Debug)]
pub struct SchemaMetadata<'a> {
    pub possible_types: PossibleTypes<'a>,
    pub type_fields: TypeFieldsMap<'a>,
}

type TypeFieldsMap<'a> = HashMap<&'a str, HashMap<&'a str, &'a str>>;

impl<'a> SchemaMetadata<'a> {
    pub fn new<'b: 'static>(schema: &'b ConsumerSchema) -> Self {
        schema_metadata(schema)
    }
}

#[derive(Debug, Default)]
pub struct PossibleTypes<'a> {
    map: HashMap<&'a str, HashSet<&'a str>>,
}

impl<'a> PossibleTypes<'a> {
    pub fn entity_satisfies_type_condition(
        &'a self,
        type_name: &'a str,
        type_condition: &'a str,
    ) -> bool {
        if type_name == type_condition {
            true
        } else if let Some(possible_types_of_type) = self.map.get(type_condition) {
            possible_types_of_type.contains(type_name)
        } else {
            false
        }
    }
    pub fn get_possible_types(&'a self, type_name: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        self.map
            .get(type_name)
            .into_iter()
            .flatten()
            .copied()
            .chain(std::iter::once(type_name))
    }

    pub fn get_possible_types_sorted(&'a self, type_name: &'a str) -> Vec<&'a str> {
        let mut list: Vec<&'a str> = self
            .map
            .get(type_name)
            .into_iter()
            .flatten()
            .copied()
            .chain(std::iter::once(type_name))
            .collect();

        list.sort_unstable();

        list
    }
}

fn schema_metadata<'a: 'static>(schema: &'a ConsumerSchema) -> SchemaMetadata<'a> {
    let mut first_possible_types: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut type_fields: HashMap<&str, HashMap<&str, &str>> = HashMap::new();

    for definition in &schema.document.definitions {
        match definition {
            Definition::TypeDefinition(TypeDefinition::Object(object_type)) => {
                let name = object_type.name.as_str();
                let fields = type_fields.entry(name).or_default();
                for field in &object_type.fields {
                    let field_type_name = field.field_type.type_name();
                    fields.insert(field.name.as_str(), field_type_name);
                }

                for interface in &object_type.implements_interfaces {
                    let interface_name = interface.as_str();
                    let possible_types_entry =
                        first_possible_types.entry(interface_name).or_default();
                    possible_types_entry.push(object_type.name.as_str());
                }
            }
            Definition::TypeDefinition(TypeDefinition::Interface(interface_type)) => {
                let name = interface_type.name.as_str();
                let mut fields = HashMap::new();
                for field in &interface_type.fields {
                    let field_type_name = field.field_type.type_name();
                    fields.insert(field.name.as_str(), field_type_name);
                }
                type_fields.insert(name, fields);
                for interface_name in &interface_type.implements_interfaces {
                    let interface_name = interface_name.as_str();
                    let possible_types_entry =
                        first_possible_types.entry(interface_name).or_default();
                    possible_types_entry.push(interface_type.name.as_str());
                }
            }
            Definition::TypeDefinition(TypeDefinition::Union(union_type)) => {
                let name = union_type.name.as_str();
                let mut types = vec![];
                for member in &union_type.types {
                    types.push(member.as_str());
                }
                first_possible_types.insert(name, types);
            }
            _ => {}
        }
    }

    let mut final_possible_types: HashMap<&str, HashSet<&str>> = HashMap::new();
    // Re-iterate over the possible_types
    for (definition_name_of_x, first_possible_types_of_x) in &first_possible_types {
        let mut possible_types_of_x: HashSet<&str> = HashSet::new();
        for definition_name_of_y in first_possible_types_of_x {
            possible_types_of_x.insert(definition_name_of_y);
            let possible_types_of_y = first_possible_types.get(definition_name_of_y);
            if let Some(possible_types_of_y) = possible_types_of_y {
                for definition_name_of_z in possible_types_of_y {
                    possible_types_of_x.insert(definition_name_of_z);
                }
            }
        }
        final_possible_types.insert(definition_name_of_x, possible_types_of_x);
    }

    SchemaMetadata {
        possible_types: PossibleTypes {
            map: final_possible_types,
        },
        type_fields,
    }
}

trait TypeName<'a> {
    fn type_name(&'a self) -> &'a str;
}

impl<'a> TypeName<'a> for Type<'a, String> {
    fn type_name(&'a self) -> &'a str {
        match self {
            graphql_parser::schema::Type::NamedType(named_type) => named_type.as_str(),
            graphql_parser::schema::Type::NonNullType(non_null_type) => non_null_type.type_name(),
            graphql_parser::schema::Type::ListType(list_type) => list_type.type_name(),
        }
    }
}
