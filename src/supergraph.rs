use std::{
    collections::{HashMap, HashSet},
    fmt::Debug,
};

use graphql_parser_hive_fork::{
    query::Directive,
    schema::{Definition, Document, Field, InterfaceType, ObjectType, TypeDefinition},
};
use graphql_tools::ast::SchemaDocumentExtension;

use crate::{
    join_field::JoinFieldDirective, join_implements::JoinImplementsDirective,
    join_type::JoinTypeDirective,
};

pub type SupergraphSchema = Document<'static, String>;

#[derive(Debug)]
pub struct SupergraphObjectType<'a> {
    pub source: &'a ObjectType<'static, String>,
    pub fields: HashMap<String, SupergraphField<'a>>,
    pub join_type: Vec<JoinTypeDirective>,
    pub join_implements: Vec<JoinImplementsDirective>,
    pub root_type: Option<RootType>,
    pub used_in_subgraphs: HashSet<String>,
}

#[derive(Debug)]
pub struct SupergraphInterfaceType<'a> {
    pub source: &'a InterfaceType<'static, String>,
    pub fields: HashMap<String, SupergraphField<'a>>,
    pub join_type: Vec<JoinTypeDirective>,
    pub join_implements: Vec<JoinImplementsDirective>,
    pub used_in_subgraphs: HashSet<String>,
}

#[derive(Debug)]
pub enum RootType {
    Query,
    Mutation,
    Subscription,
}

#[derive(Debug)]
pub struct SupergraphField<'a> {
    pub source: &'a Field<'static, String>,
    pub join_field: Vec<JoinFieldDirective>,
}

#[derive(Debug)]
pub enum SupergraphDefinition<'a> {
    Object(SupergraphObjectType<'a>),
    Interface(SupergraphInterfaceType<'a>),
}

impl<'a> SupergraphDefinition<'a> {
    pub fn name(&self) -> &str {
        match self {
            SupergraphDefinition::Object(object_type) => &object_type.source.name,
            SupergraphDefinition::Interface(interface_type) => &interface_type.source.name,
        }
    }

    pub fn available_in_subgraph(&self, subgraph: &str) -> bool {
        match self {
            SupergraphDefinition::Object(object_type) => {
                object_type.used_in_subgraphs.contains(subgraph)
            }
            SupergraphDefinition::Interface(interface_type) => {
                interface_type.used_in_subgraphs.contains(subgraph)
            }
        }
    }

    pub fn is_root(&self) -> bool {
        match self {
            SupergraphDefinition::Object(object_type) => object_type.root_type.is_some(),
            _ => false,
        }
    }

    pub fn root_type(&self) -> Option<&RootType> {
        match self {
            SupergraphDefinition::Object(object_type) => object_type.root_type.as_ref(),
            _ => None,
        }
    }

    pub fn fields(&self) -> &HashMap<String, SupergraphField> {
        match self {
            SupergraphDefinition::Object(object_type) => &object_type.fields,
            SupergraphDefinition::Interface(interface_type) => &interface_type.fields,
        }
    }

    pub fn join_types(&self) -> &Vec<JoinTypeDirective> {
        match self {
            SupergraphDefinition::Object(object_type) => &object_type.join_type,
            SupergraphDefinition::Interface(interface_type) => &interface_type.join_type,
        }
    }

    pub fn join_implements(&self) -> &Vec<JoinImplementsDirective> {
        match self {
            SupergraphDefinition::Object(object_type) => &object_type.join_implements,
            SupergraphDefinition::Interface(interface_type) => &interface_type.join_implements,
        }
    }
}

#[derive(Debug)]
pub struct SupergraphIR<'a> {
    pub definitions: HashMap<String, SupergraphDefinition<'a>>,
}

impl<'a> SupergraphIR<'a> {
    pub fn new(schema: &'a SupergraphSchema) -> Self {
        Self {
            definitions: Self::build_map(schema),
        }
    }

    fn build_map(schema: &'a SupergraphSchema) -> HashMap<String, SupergraphDefinition<'a>> {
        schema
            .definitions
            .iter()
            .filter_map(|definition| match definition {
                Definition::TypeDefinition(TypeDefinition::Object(object_type)) => Some((
                    object_type.name.to_string(),
                    SupergraphDefinition::Object(Self::build_object_type(object_type, schema)),
                )),
                Definition::TypeDefinition(TypeDefinition::Interface(interface_type)) => Some((
                    interface_type.name.to_string(),
                    SupergraphDefinition::Interface(Self::build_interface_type(interface_type)),
                )),
                _ => None,
            })
            .collect()
    }

    fn build_fields(fields: &'a [Field<'static, String>]) -> HashMap<String, SupergraphField<'a>> {
        fields
            .iter()
            .map(|field| {
                (
                    field.name.to_string(),
                    SupergraphField {
                        source: field,
                        join_field: Self::extract_join_field_from_directives(&field.directives),
                    },
                )
            })
            .collect()
    }

    fn build_interface_type(
        interface_type: &'a InterfaceType<'static, String>,
    ) -> SupergraphInterfaceType<'a> {
        let fields = Self::build_fields(&interface_type.fields);
        let used_in_subgraphs = Self::build_subgraph_usage_from_fields(&fields);

        SupergraphInterfaceType {
            source: interface_type,
            fields,
            join_type: Self::extract_join_types_from_directives(&interface_type.directives),
            join_implements: Self::extract_join_implements_from_directives(
                &interface_type.directives,
            ),
            used_in_subgraphs,
        }
    }

    fn build_object_type(
        object_type: &'a ObjectType<'static, String>,
        schema: &'a SupergraphSchema,
    ) -> SupergraphObjectType<'a> {
        let fields = Self::build_fields(&object_type.fields);

        let root_type = if object_type.name == schema.query_type().name {
            Some(RootType::Query)
        } else if schema
            .mutation_type()
            .is_some_and(|t| t.name == object_type.name)
        {
            Some(RootType::Mutation)
        } else if schema
            .subscription_type()
            .is_some_and(|t| t.name == object_type.name)
        {
            Some(RootType::Subscription)
        } else {
            None
        };

        let used_in_subgraphs = Self::build_subgraph_usage_from_fields(&fields);

        SupergraphObjectType {
            source: object_type,
            fields,
            join_type: Self::extract_join_types_from_directives(&object_type.directives),
            join_implements: Self::extract_join_implements_from_directives(&object_type.directives),
            root_type,
            used_in_subgraphs,
        }
    }

    fn build_subgraph_usage_from_fields(
        fields: &HashMap<String, SupergraphField>,
    ) -> HashSet<String> {
        fields
            .iter()
            .flat_map(|(_field_name, field)| field.join_field.iter())
            .filter_map(|join_field| join_field.graph.as_ref().map(|graph| graph.to_string()))
            .collect()
    }

    fn extract_join_field_from_directives(
        directives: &[Directive<'static, String>],
    ) -> Vec<JoinFieldDirective> {
        directives
            .iter()
            .filter_map(|directive| {
                if JoinFieldDirective::is(directive) {
                    Some(JoinFieldDirective::from(directive))
                } else {
                    None
                }
            })
            .collect()
    }

    fn extract_join_implements_from_directives(
        directives: &[Directive<'static, String>],
    ) -> Vec<JoinImplementsDirective> {
        directives
            .iter()
            .filter_map(|directive| {
                if JoinImplementsDirective::is(directive) {
                    Some(JoinImplementsDirective::from(directive))
                } else {
                    None
                }
            })
            .collect()
    }

    fn extract_join_types_from_directives(
        directives: &[Directive<'static, String>],
    ) -> Vec<JoinTypeDirective> {
        directives
            .iter()
            .filter_map(|directive| {
                if JoinTypeDirective::is(directive) {
                    Some(JoinTypeDirective::from(directive))
                } else {
                    None
                }
            })
            .collect()
    }
}
