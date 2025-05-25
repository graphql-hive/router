use std::collections::{HashMap, HashSet};

use graphql_tools::ast::TypeExtension;
use tracing::instrument;

use crate::federation_spec::{directives::JoinFieldDirective, join_type::JoinTypeDirective};

use super::supergraph_state::{
    SupergraphDefinition, SupergraphInterfaceType, SupergraphObjectType, SupergraphState,
};

pub type SubgraphId = String;

#[derive(Debug)]
pub struct SubgraphState {
    pub graph_id: SubgraphId,
    pub definitions: HashMap<String, SubgraphDefinition>,
    pub entity_type_names: HashSet<String>,
}

impl SubgraphState {
    #[instrument(skip(supergraph_state))]
    pub fn decompose_from_supergraph(
        graph_id: &SubgraphId,
        supergraph_state: &SupergraphState,
    ) -> Self {
        let mut instance = Self {
            graph_id: graph_id.clone(),
            definitions: HashMap::new(),
            entity_type_names: HashSet::new(),
        };

        for (def_name, supergraph_def) in supergraph_state.definitions.iter() {
            let relevant_join_types = supergraph_def.extract_join_types_for(graph_id);

            if relevant_join_types.is_empty() {
                continue;
            }

            let subgraph_def = match supergraph_def {
                SupergraphDefinition::Object(supergraph_object_type) => Self::process_object_type(
                    graph_id,
                    &relevant_join_types,
                    supergraph_object_type,
                ),
                SupergraphDefinition::Interface(supergraph_interface_type) => {
                    Self::process_interface_type(
                        graph_id,
                        &relevant_join_types,
                        supergraph_interface_type,
                    )
                }
                SupergraphDefinition::Enum(enum_type) => {
                    Self::process_enum_type(graph_id, &relevant_join_types, enum_type)
                }
                SupergraphDefinition::Union(union_type) => {
                    Self::process_union_type(graph_id, &relevant_join_types, union_type)
                }
                SupergraphDefinition::Scalar(scalar_type) => Self::process_scalar_type(scalar_type),
                SupergraphDefinition::InputObject(scalar_type) => {
                    Self::process_input_object_type(graph_id, &relevant_join_types, scalar_type)
                }
            };

            if let Some(subgraph_def) = subgraph_def {
                if subgraph_def.is_entity_type() {
                    instance.entity_type_names.insert(def_name.clone());
                }

                instance.definitions.insert(def_name.clone(), subgraph_def);
            }
        }

        instance
    }

    fn process_object_type(
        graph_id: &str,
        graph_join_types: &[JoinTypeDirective],
        supergraph_object_type: &SupergraphObjectType<'_>,
    ) -> Option<SubgraphDefinition> {
        let relevant_fields: Vec<SubgraphField> = supergraph_object_type
            .fields_of_subgraph(graph_id)
            .iter()
            .map(
                |(field_name, (field_def, maybe_join_field))| SubgraphField {
                    name: field_name.to_string(),
                    join_field: maybe_join_field.clone(),
                    return_type_name: field_def.source.field_type.inner_type().to_string(),
                    is_list: field_def.source.field_type.is_list_type(),
                },
            )
            .collect();

        if relevant_fields.is_empty() {
            return None;
        }

        let subgraph_obj_type = SubgraphDefinition::Object(SubgraphObjectType {
            name: supergraph_object_type.source.name.to_string(),
            fields: relevant_fields,
            join_types: graph_join_types.to_owned(),
        });

        Some(subgraph_obj_type)
    }

    #[cfg(test)]
    pub fn known_subgraph_definitions(&self) -> HashMap<&String, &SubgraphDefinition> {
        self.definitions.iter().collect()
    }

    fn process_interface_type(
        graph_id: &str,
        graph_join_types: &[JoinTypeDirective],
        supergraph_interface_type: &SupergraphInterfaceType<'_>,
    ) -> Option<SubgraphDefinition> {
        let relevant_fields: Vec<SubgraphField> = supergraph_interface_type
            .fields_of_subgraph(graph_id)
            .iter()
            .map(
                |(field_name, (field_def, maybe_join_field))| SubgraphField {
                    name: field_name.to_string(),
                    join_field: maybe_join_field.clone(),
                    return_type_name: field_def.source.field_type.inner_type().to_string(),
                    is_list: field_def.source.field_type.is_list_type(),
                },
            )
            .collect();

        if relevant_fields.is_empty() {
            return None;
        }

        let subgraph_interface_type = SubgraphDefinition::Interface(SubgraphInterfaceType {
            name: supergraph_interface_type.source.name.to_string(),
            fields: relevant_fields,
            join_types: graph_join_types.to_owned(),
        });

        Some(subgraph_interface_type)
    }

    fn process_enum_type(
        graph_id: &str,
        graph_join_types: &[JoinTypeDirective],
        enum_type: &super::supergraph_state::SupergraphEnumType<'_>,
    ) -> Option<SubgraphDefinition> {
        let relevant_values = enum_type.values_of_subgraph(graph_id);

        if relevant_values.is_empty() {
            return None;
        }

        let values = relevant_values
            .iter()
            .map(|value| SubgraphEnumValueType {
                name: value.source.name.to_string(),
            })
            .collect();

        Some(SubgraphDefinition::Enum(SubgraphEnumType {
            name: enum_type.source.name.to_string(),
            values,
            join_types: graph_join_types.to_owned(),
        }))
    }

    fn process_union_type(
        graph_id: &str,
        graph_join_types: &[JoinTypeDirective],
        union_type: &super::supergraph_state::SupergraphUnionType<'_>,
    ) -> Option<SubgraphDefinition> {
        let relevant_types = union_type.relevant_types(graph_id);

        if relevant_types.is_empty() {
            return None;
        }

        Some(SubgraphDefinition::Union(SubgraphUnionType {
            name: union_type.source.name.to_string(),
            types: relevant_types.iter().map(|v| v.to_string()).collect(),
            join_types: graph_join_types.to_owned(),
        }))
    }

    fn process_scalar_type(
        scalar_type: &super::supergraph_state::SupergraphScalarType<'_>,
    ) -> Option<SubgraphDefinition> {
        Some(SubgraphDefinition::Scalar(SubgraphScalarType {
            name: scalar_type.source.name.to_string(),
        }))
    }

    fn process_input_object_type(
        _graph_id: &str,
        _graph_join_types: &[JoinTypeDirective],
        _scalar_type: &super::supergraph_state::SupergraphInputObjectType<'_>,
    ) -> Option<SubgraphDefinition> {
        //unimplemented!("not there yet")
        println!("not there yet");
        None
    }
}

#[derive(Debug)]
pub enum SubgraphDefinition {
    Object(SubgraphObjectType),
    Interface(SubgraphInterfaceType),
    Enum(SubgraphEnumType),
    Union(SubgraphUnionType),
    Scalar(SubgraphScalarType),
    InputObject(SubgraphInputObjectType),
}

impl SubgraphDefinition {
    pub fn fields(&self) -> Option<&Vec<SubgraphField>> {
        match self {
            SubgraphDefinition::Object(obj) => Some(&obj.fields),
            SubgraphDefinition::Interface(iface) => Some(&iface.fields),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub struct SubgraphEnumType {
    pub name: String,
    pub values: Vec<SubgraphEnumValueType>,
    pub join_types: Vec<JoinTypeDirective>,
}

#[derive(Debug)]
pub struct SubgraphEnumValueType {
    pub name: String,
}

#[derive(Debug)]
pub struct SubgraphScalarType {
    pub name: String,
}

#[derive(Debug)]
pub struct SubgraphInputObjectType {
    pub name: String,
}

#[derive(Debug)]
pub struct SubgraphUnionType {
    pub name: String,
    pub types: Vec<String>,
    pub join_types: Vec<JoinTypeDirective>,
}

impl SubgraphDefinition {
    pub fn name(&self) -> &str {
        match self {
            SubgraphDefinition::Object(obj) => &obj.name,
            SubgraphDefinition::Interface(iface) => &iface.name,
            SubgraphDefinition::Union(union) => &union.name,
            SubgraphDefinition::Enum(enum_type) => &enum_type.name,
            SubgraphDefinition::Scalar(scalar) => &scalar.name,
            SubgraphDefinition::InputObject(input_object) => &input_object.name,
        }
    }

    pub fn is_entity_type(&self) -> bool {
        match self {
            SubgraphDefinition::Object(obj) => obj
                .join_types
                .iter()
                .any(|jt| matches!((&jt.resolvable, &jt.key), (true, Some(_key)))),
            _ => false,
        }
    }
}

#[derive(Debug)]
pub struct SubgraphObjectType {
    pub name: String,
    pub fields: Vec<SubgraphField>,
    pub join_types: Vec<JoinTypeDirective>,
}

#[derive(Debug)]
pub struct SubgraphInterfaceType {
    pub name: String,
    pub fields: Vec<SubgraphField>,
    pub join_types: Vec<JoinTypeDirective>,
}

#[derive(Debug)]
pub struct SubgraphField {
    pub name: String,
    pub return_type_name: String,
    pub is_list: bool,
    pub join_field: Option<JoinFieldDirective>,
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::utils::parsing::parse_schema;

    use super::*;

    #[test]
    fn decompose_supergraph_into_subgraphs() {
        let supergraph_path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixture/supergraph.graphql");
        let supergraph_sdl =
            &std::fs::read_to_string(supergraph_path).expect("Unable to read input file");
        let schema = parse_schema(supergraph_sdl);
        let supergraph = SupergraphState::new(&schema);

        assert_eq!(supergraph.subgraphs_state.keys().count(), 5);
        assert!(supergraph.subgraphs_state.contains_key("PANDAS"));
        assert!(supergraph.subgraphs_state.contains_key("USERS"));
        assert!(supergraph.subgraphs_state.contains_key("REVIEWS"));
        assert!(supergraph.subgraphs_state.contains_key("PRODUCTS"));
        assert!(supergraph.subgraphs_state.contains_key("INVENTORY"));

        let types = supergraph
            .subgraph_state("PANDAS")
            .expect("failed to find subgraph")
            .known_subgraph_definitions();

        assert_eq!(types.len(), 2); // Query, Panda
        let mut query_type_fields = types
            .get(&String::from("Query"))
            .expect("Query type not found")
            .fields()
            .unwrap()
            .iter()
            .map(|r| &r.name)
            .collect::<Vec<_>>();
        query_type_fields.sort();
        assert_eq!(query_type_fields.len(), 2);
        assert_eq!(query_type_fields, vec!["allPandas", "panda"]);

        let mut panda_type_fields = types
            .get(&String::from("Panda"))
            .expect("Panda type not found")
            .fields()
            .unwrap()
            .iter()
            .map(|r| &r.name)
            .collect::<Vec<_>>();
        panda_type_fields.sort();
        assert_eq!(panda_type_fields.len(), 2);
        assert_eq!(panda_type_fields, vec!["favoriteFood", "name"]);

        let types = supergraph
            .subgraph_state("USERS")
            .expect("failed to find subgraph")
            .known_subgraph_definitions();

        assert_eq!(types.len(), 1);
        assert!(!types.contains_key(&String::from("Query")));
        let mut user_type_fields = types
            .get(&String::from("User"))
            .expect("User type not found")
            .fields()
            .unwrap()
            .iter()
            .map(|r| &r.name)
            .collect::<Vec<_>>();
        user_type_fields.sort();
        assert_eq!(user_type_fields.len(), 3);
        assert_eq!(
            user_type_fields,
            vec!["email", "name", "totalProductsCreated"]
        );

        let types = supergraph
            .subgraph_state("REVIEWS")
            .expect("failed to find subgraph")
            .known_subgraph_definitions();
        assert_eq!(types.len(), 4);
        let mut product_type_fields = types
            .get(&String::from("Product"))
            .expect("Product type not found")
            .fields()
            .unwrap()
            .iter()
            .map(|r| &r.name)
            .collect::<Vec<_>>();
        product_type_fields.sort();
        assert_eq!(product_type_fields.len(), 4);
        assert_eq!(
            product_type_fields,
            vec!["id", "reviews", "reviewsCount", "reviewsScore"]
        );

        let types = supergraph
            .subgraph_state("PRODUCTS")
            .expect("failed to find subgraph")
            .known_subgraph_definitions();
        assert_eq!(types.len(), 8);
        let mut query_type_fields = types
            .get(&String::from("Query"))
            .expect("Query type not found")
            .fields()
            .unwrap()
            .iter()
            .map(|r| &r.name)
            .collect::<Vec<_>>();
        query_type_fields.sort();
        assert_eq!(query_type_fields.len(), 2);
        assert_eq!(query_type_fields, vec!["allProducts", "product"]);
        let mut product_type_fields = types
            .get(&String::from("Product"))
            .expect("Product type not found")
            .fields()
            .unwrap()
            .iter()
            .map(|r| &r.name)
            .collect::<Vec<_>>();
        product_type_fields.sort();
        assert_eq!(product_type_fields.len(), 9);

        assert_eq!(
            product_type_fields,
            vec![
                "createdBy",
                "dimensions",
                "hidden",
                "id",
                "name",
                "oldField",
                "package",
                "sku",
                "variation"
            ]
        );

        let types = supergraph
            .subgraph_state("INVENTORY")
            .expect("failed to find subgraph")
            .known_subgraph_definitions();

        assert_eq!(types.len(), 5); // Inventory
    }
}
