use std::{
    collections::{HashMap, HashSet},
    fmt::Debug,
};

use graphql_parser_hive_fork::{
    query::Directive,
    schema::{Definition, Document, Field, InterfaceType, ObjectType, TypeDefinition},
};
use graphql_tools::ast::{SchemaDocumentExtension, TypeExtension};

use crate::federation_spec::directives::{
    InaccessibleDirective, JoinFieldDirective, JoinImplementsDirective, JoinTypeDirective,
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

impl<'a> SupergraphObjectType<'a> {
    pub fn available_in_subgraph(&self, subgraph: &str) -> bool {
        // First check join_type directives
        let available_in_join_type = self.join_type.iter().any(|jt| jt.graph == subgraph);

        if available_in_join_type {
            return true;
        }

        // Then check fields
        self.used_in_fields(subgraph)
    }

    pub fn used_in_fields(&self, subgraph: &str) -> bool {
        self.used_in_subgraphs.contains(subgraph)
    }
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
    pub is_scalar: bool,
    pub inaccessible: bool,
    pub join_field: Vec<JoinFieldDirective>,
}

#[derive(Debug)]
pub enum SupergraphDefinition<'a> {
    Object(SupergraphObjectType<'a>),
    Interface(SupergraphInterfaceType<'a>),
}

impl SupergraphDefinition<'_> {
    pub fn name(&self) -> &str {
        match self {
            SupergraphDefinition::Object(object_type) => &object_type.source.name,
            SupergraphDefinition::Interface(interface_type) => &interface_type.source.name,
        }
    }

    pub fn available_in_subgraph(&self, subgraph: &str) -> bool {
        // First check join_type directives
        let available_in_join_type = self.join_types().iter().any(|jt| jt.graph == subgraph);

        if available_in_join_type {
            return true;
        }

        // Then check fields
        self.used_in_fields(subgraph)
    }

    pub fn is_interface(&self) -> bool {
        match self {
            SupergraphDefinition::Object(_) => false,
            SupergraphDefinition::Interface(_) => true,
        }
    }

    pub fn used_in_fields(&self, subgraph: &str) -> bool {
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

    pub fn subgraphs(&self) -> Vec<&str> {
        match self {
            SupergraphDefinition::Object(object_type) => object_type
                .join_type
                .iter()
                .map(|join_type| join_type.graph.as_str())
                .collect::<Vec<&str>>(),
            SupergraphDefinition::Interface(interface_type) => interface_type
                .join_type
                .iter()
                .map(|join_type| join_type.graph.as_str())
                .collect::<Vec<&str>>(),
        }
    }

    pub fn join_implements(&self) -> &Vec<JoinImplementsDirective> {
        match self {
            SupergraphDefinition::Object(object_type) => &object_type.join_implements,
            SupergraphDefinition::Interface(interface_type) => &interface_type.join_implements,
        }
    }

    // Check if the given type name represents a scalar type
    pub fn is_scalar_type(&self, type_name: &str) -> bool {
        // Standard GraphQL scalar types
        static STANDARD_SCALARS: [&str; 5] = ["String", "Int", "Float", "Boolean", "ID"];
        STANDARD_SCALARS.contains(&type_name)
    }
}

#[derive(Debug)]
pub struct SupergraphMetadata<'a> {
    pub definitions: HashMap<String, SupergraphDefinition<'a>>,
    pub document: &'a Document<'static, String>,
}

impl<'a> SupergraphMetadata<'a> {
    pub fn new(schema: &'a SupergraphSchema) -> Self {
        Self {
            document: schema,
            definitions: Self::build_map(schema),
        }
    }

    // Helper method to check if a type is a scalar
    pub fn is_scalar_type(&self, type_name: &str) -> bool {
        // Standard GraphQL scalar types
        static STANDARD_SCALARS: [&str; 5] = ["String", "Int", "Float", "Boolean", "ID"];

        if STANDARD_SCALARS.contains(&type_name) {
            return true;
        }

        // Check for custom scalar types in the schema
        self.document.definitions.iter().any(|def| {
            if let Definition::TypeDefinition(TypeDefinition::Scalar(scalar)) = def {
                scalar.name == type_name
            } else {
                false
            }
        })
    }

    fn build_map(schema: &'a SupergraphSchema) -> HashMap<String, SupergraphDefinition<'a>> {
        let known_scalars = [
            schema
                .definitions
                .iter()
                .filter_map(|def| match def {
                    Definition::TypeDefinition(TypeDefinition::Scalar(scalar_type)) => {
                        Some(scalar_type.name.as_str())
                    }
                    _ => None,
                })
                .collect::<Vec<&str>>(),
            vec!["ID", "String", "Boolean", "Int", "Float"],
        ]
        .concat();

        schema
            .definitions
            .iter()
            .filter_map(|definition| match definition {
                Definition::TypeDefinition(TypeDefinition::Object(object_type)) => Some((
                    object_type.name.to_string(),
                    SupergraphDefinition::Object(Self::build_object_type(
                        object_type,
                        schema,
                        &known_scalars,
                    )),
                )),
                Definition::TypeDefinition(TypeDefinition::Interface(interface_type)) => Some((
                    interface_type.name.to_string(),
                    SupergraphDefinition::Interface(Self::build_interface_type(
                        interface_type,
                        &known_scalars,
                    )),
                )),
                _ => None,
            })
            .collect()
    }

    fn build_fields(
        fields: &'a [Field<'static, String>],
        known_scalars: &Vec<&'a str>,
    ) -> HashMap<String, SupergraphField<'a>> {
        fields
            .iter()
            .map(|field| {
                (
                    field.name.to_string(),
                    SupergraphField {
                        source: field,
                        is_scalar: known_scalars.contains(&field.field_type.inner_type()),
                        join_field: Self::extract_join_field_from_directives(&field.directives),
                        inaccessible: !Self::extract_inaccessible_from_directives(
                            &field.directives,
                        )
                        .is_empty(),
                    },
                )
            })
            .collect()
    }

    fn build_interface_type(
        interface_type: &'a InterfaceType<'static, String>,
        known_scalars: &Vec<&'a str>,
    ) -> SupergraphInterfaceType<'a> {
        let fields = Self::build_fields(&interface_type.fields, known_scalars);
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
        known_scalars: &Vec<&'a str>,
    ) -> SupergraphObjectType<'a> {
        let fields = Self::build_fields(&object_type.fields, &known_scalars);

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
        let mut subgraphs = HashSet::new();

        // Add subgraphs from join_field directives
        for (_field_name, field) in fields.iter() {
            for join_field in field.join_field.iter() {
                if let Some(graph) = &join_field.graph {
                    subgraphs.insert(graph.to_string());
                }
            }
        }

        subgraphs
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

    fn extract_inaccessible_from_directives(
        directives: &[Directive<'static, String>],
    ) -> Vec<InaccessibleDirective> {
        directives
            .iter()
            .filter_map(|directive| {
                if InaccessibleDirective::is(directive) {
                    Some(InaccessibleDirective::from(directive))
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
