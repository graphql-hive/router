use std::{
    collections::{HashMap, HashSet},
    fmt::Debug,
};

use graphql_parser_hive_fork::{
    query::Directive,
    schema::{
        Definition, Document, EnumType, EnumValue, Field, InputObjectType, InterfaceType,
        ObjectType, ScalarType, TypeDefinition, UnionType,
    },
};
use graphql_tools::ast::SchemaDocumentExtension;

use crate::federation_spec::directives::{
    JoinEnumValueDirective, JoinFieldDirective, JoinGraphDirective, JoinImplementsDirective,
    JoinTypeDirective, JoinUnionMemberDirective,
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
    Union(SupergraphUnionType<'a>),
    Enum(SupergraphEnumType<'a>),
    Scalar(SupergraphScalarType<'a>),
    InputObject(SupergraphInputObjectType<'a>),
}

#[derive(Debug)]
pub struct SupergraphEnumValueType<'a> {
    pub source: &'a EnumValue<'static, String>,
    pub join_enum_value: Vec<JoinEnumValueDirective>,
}

#[derive(Debug)]
pub struct SupergraphInputObjectType<'a> {
    pub source: &'a InputObjectType<'static, String>,
    pub join_type: Vec<JoinTypeDirective>,
}

#[derive(Debug)]
pub struct SupergraphScalarType<'a> {
    pub source: &'a ScalarType<'static, String>,
    pub join_type: Vec<JoinTypeDirective>,
}

#[derive(Debug)]
pub struct SupergraphEnumType<'a> {
    pub source: &'a EnumType<'static, String>,
    pub values: Vec<SupergraphEnumValueType<'a>>,
    pub join_type: Vec<JoinTypeDirective>,
}

#[derive(Debug)]
pub struct SupergraphUnionType<'a> {
    pub source: &'a UnionType<'static, String>,
    pub join_type: Vec<JoinTypeDirective>,
    pub union_members: Vec<JoinUnionMemberDirective>,
}

impl SupergraphDefinition<'_> {
    pub fn name(&self) -> &str {
        match self {
            SupergraphDefinition::Object(object_type) => &object_type.source.name,
            SupergraphDefinition::Interface(interface_type) => &interface_type.source.name,
            SupergraphDefinition::Union(union_type) => &union_type.source.name,
            SupergraphDefinition::Enum(enum_type) => &enum_type.source.name,
            SupergraphDefinition::Scalar(scalar_type) => &scalar_type.source.name,
            SupergraphDefinition::InputObject(input_type) => &input_type.source.name,
        }
    }

    pub fn is_defined_in_subgraph(&self, subgraph: &str) -> bool {
        self.join_types().iter().any(|jt| jt.graph == subgraph)
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
            SupergraphDefinition::Interface(_) => true,
            _ => false,
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
            _ => false,
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
        static EMPTY: std::sync::LazyLock<HashMap<String, SupergraphField>> =
            std::sync::LazyLock::new(|| HashMap::<String, SupergraphField>::new());

        match self {
            SupergraphDefinition::Object(object_type) => &object_type.fields,
            SupergraphDefinition::Interface(interface_type) => &interface_type.fields,
            _ => &EMPTY,
        }
    }

    pub fn join_types(&self) -> &Vec<JoinTypeDirective> {
        match self {
            SupergraphDefinition::Object(object_type) => &object_type.join_type,
            SupergraphDefinition::Interface(interface_type) => &interface_type.join_type,
            SupergraphDefinition::Union(_)
            | SupergraphDefinition::Enum(_)
            | SupergraphDefinition::Scalar(_)
            | SupergraphDefinition::InputObject(_) => {
                static EMPTY: Vec<JoinTypeDirective> = Vec::new();
                &EMPTY
            }
        }
    }

    pub fn join_implements(&self) -> &Vec<JoinImplementsDirective> {
        match self {
            SupergraphDefinition::Object(object_type) => &object_type.join_implements,
            SupergraphDefinition::Interface(interface_type) => &interface_type.join_implements,
            SupergraphDefinition::Union(_)
            | SupergraphDefinition::Enum(_)
            | SupergraphDefinition::Scalar(_)
            | SupergraphDefinition::InputObject(_) => {
                static EMPTY: Vec<JoinImplementsDirective> = Vec::new();
                &EMPTY
            }
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
    pub known_subgraphs: HashSet<String>,
}

impl<'a> SupergraphMetadata<'a> {
    pub fn new(schema: &'a SupergraphSchema) -> Self {
        Self {
            document: schema,
            definitions: Self::build_map(schema),
            known_subgraphs: Self::extract_subgraph_names(schema),
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

    fn extract_subgraph_names(schema: &'a SupergraphSchema) -> HashSet<String> {
        let mut set = HashSet::new();
        let join_graph_enum = schema.definitions.iter().find_map(|d| match d {
            Definition::TypeDefinition(TypeDefinition::Enum(e)) => {
                if e.name == "join__Graph" {
                    Some(e)
                } else {
                    None
                }
            }
            _ => None,
        });

        if let Some(join_graph_enum) = join_graph_enum {
            for value in join_graph_enum.values.iter() {
                let join_graph_directive: Option<JoinGraphDirective> = value
                    .directives
                    .iter()
                    .find(|d| JoinGraphDirective::is(d))
                    .map(|d| d.into());

                if let Some(join_graph_directive) = join_graph_directive {
                    set.insert(join_graph_directive.name);
                }
            }
        }

        set
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
                Definition::TypeDefinition(TypeDefinition::Enum(enum_type)) => Some((
                    enum_type.name.to_string(),
                    SupergraphDefinition::Enum(Self::build_enum_type(enum_type)),
                )),
                Definition::TypeDefinition(TypeDefinition::Union(union_type)) => Some((
                    union_type.name.to_string(),
                    SupergraphDefinition::Union(Self::build_union_type(union_type)),
                )),
                Definition::TypeDefinition(TypeDefinition::Scalar(scalar_type)) => Some((
                    scalar_type.name.to_string(),
                    SupergraphDefinition::Scalar(Self::build_scalar_type(scalar_type)),
                )),
                Definition::TypeDefinition(TypeDefinition::InputObject(input_object_type)) => {
                    Some((
                        input_object_type.name.to_string(),
                        SupergraphDefinition::InputObject(Self::build_input_object_type(
                            input_object_type,
                        )),
                    ))
                }
                _ => None,
            })
            .collect()
    }

    fn build_input_object_type(
        input_object_type: &'a InputObjectType<'static, String>,
    ) -> SupergraphInputObjectType<'a> {
        SupergraphInputObjectType {
            source: input_object_type,
            join_type: Self::extract_join_types_from_directives(&input_object_type.directives),
        }
    }

    fn build_scalar_type(scalar_type: &'a ScalarType<'static, String>) -> SupergraphScalarType<'a> {
        SupergraphScalarType {
            source: scalar_type,
            join_type: Self::extract_join_types_from_directives(&scalar_type.directives),
        }
    }

    fn build_union_type(union_type: &'a UnionType<'static, String>) -> SupergraphUnionType<'a> {
        SupergraphUnionType {
            source: union_type,
            join_type: Self::extract_join_types_from_directives(&union_type.directives),
            union_members: Self::extract_join_union_from_directives(&union_type.directives),
        }
    }

    fn build_enum_type(enum_type: &'a EnumType<'static, String>) -> SupergraphEnumType<'a> {
        SupergraphEnumType {
            source: enum_type,
            join_type: Self::extract_join_types_from_directives(&enum_type.directives),
            values: enum_type
                .values
                .iter()
                .map(|value| SupergraphEnumValueType {
                    source: value,
                    join_enum_value: Self::extract_join_enum_value_from_directives(
                        &value.directives,
                    ),
                })
                .collect(),
        }
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

    fn extract_join_union_from_directives(
        directives: &[Directive<'static, String>],
    ) -> Vec<JoinUnionMemberDirective> {
        directives
            .iter()
            .filter_map(|directive| {
                if JoinUnionMemberDirective::is(directive) {
                    Some(JoinUnionMemberDirective::from(directive))
                } else {
                    None
                }
            })
            .collect()
    }

    fn extract_join_enum_value_from_directives(
        directives: &[Directive<'static, String>],
    ) -> Vec<JoinEnumValueDirective> {
        directives
            .iter()
            .filter_map(|directive| {
                if JoinEnumValueDirective::is(directive) {
                    Some(JoinEnumValueDirective::from(directive))
                } else {
                    None
                }
            })
            .collect()
    }
}
