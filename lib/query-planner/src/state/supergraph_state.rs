use std::{
    collections::{HashMap, HashSet},
    fmt::{Debug, Display},
};

use graphql_parser_hive_fork::query::Directive;
use graphql_parser_hive_fork::schema as input;
use graphql_tools::ast::SchemaDocumentExtension;
use tracing::instrument;

use crate::federation_spec::directives::{
    FederationDirective, InaccessibleDirective, JoinEnumValueDirective, JoinFieldDirective,
    JoinGraphDirective, JoinImplementsDirective, JoinTypeDirective, JoinUnionMemberDirective,
};

use super::{selection_resolver::SelectionResolver, subgraph_state::SubgraphState};

static BUILDIB_SCALARS: [&str; 5] = ["String", "Int", "Float", "Boolean", "ID"];

pub type SchemaDocument = input::Document<'static, String>;

#[derive(Debug, thiserror::Error)]
pub enum SupergraphStateError {
    #[error("Subgraph not found: '{0}'")]
    SubgraphNotFound(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SubgraphName(pub String);

impl Display for SubgraphName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl SubgraphName {
    pub fn any() -> Self {
        Self("*".to_string())
    }
}

#[derive(Debug)]
pub struct SupergraphState<'a> {
    /// A map all of definitions (def_name, def) that exists in the schema.
    pub definitions: HashMap<String, SupergraphDefinition<'a>>,
    /// The original schema document passed to the state processor
    pub document: &'a input::Document<'static, String>,
    /// A map of (SUBGRAPH_ID, subgraph_name) to make it easy to resolve
    pub known_subgraphs: HashMap<String, String>,
    /// A set of all known scalars in this schema, including built-ins
    pub known_scalars: HashSet<String>,
    /// A map from subgraph id to a subgraph state
    pub subgraphs_state: HashMap<String, SelectionResolver>,
}

impl<'a> SupergraphState<'a> {
    #[instrument(skip(schema), name = "new_supergraph_state")]
    pub fn new(schema: &'a SchemaDocument) -> Self {
        let mut instance = Self {
            document: schema,
            definitions: Self::build_map(schema),
            known_subgraphs: Self::extract_subgraph_names(schema),
            known_scalars: Self::extract_known_scalars(schema),
            subgraphs_state: HashMap::new(),
        };

        for subgraph_id in instance.known_subgraphs.keys() {
            let state = SubgraphState::decompose_from_supergraph(subgraph_id, &instance);
            let resolver = SelectionResolver::new_from_state(state);

            instance
                .subgraphs_state
                .insert(subgraph_id.clone(), resolver);
        }

        instance
    }

    pub fn resolve_graph_id(&self, graph_id: &str) -> Result<SubgraphName, SupergraphStateError> {
        self.known_subgraphs
            .get(graph_id)
            .map(|subgraph_name| SubgraphName(subgraph_name.clone()))
            .ok_or_else(|| SupergraphStateError::SubgraphNotFound(graph_id.to_string()))
    }

    pub fn subgraph_state(
        &self,
        subgraph_id: &str,
    ) -> Result<&SubgraphState, SupergraphStateError> {
        self.subgraphs_state
            .get(subgraph_id)
            .ok_or_else(|| SupergraphStateError::SubgraphNotFound(subgraph_id.to_string()))
            .map(|v| &v.subgraph_state)
    }

    pub fn selection_resolvers_for_subgraph(
        &self,
        subgraph_id: &str,
    ) -> Result<&SelectionResolver, SupergraphStateError> {
        self.subgraphs_state
            .get(subgraph_id)
            .ok_or_else(|| SupergraphStateError::SubgraphNotFound(subgraph_id.to_string()))
    }

    pub fn is_scalar_type(&self, type_name: &str) -> bool {
        if BUILDIB_SCALARS.contains(&type_name) {
            return true;
        }

        self.document.definitions.iter().any(|def| {
            if let input::Definition::TypeDefinition(input::TypeDefinition::Scalar(scalar)) = def {
                scalar.name == type_name
            } else {
                false
            }
        })
    }

    fn extract_known_scalars(schema: &'a SchemaDocument) -> HashSet<String> {
        let mut set = HashSet::new();

        for def in schema.definitions.iter() {
            if let input::Definition::TypeDefinition(input::TypeDefinition::Scalar(scalar_type)) =
                def
            {
                set.insert(scalar_type.name.to_string());
            }
        }

        for builtin in BUILDIB_SCALARS {
            set.insert(builtin.to_string());
        }

        set
    }

    fn extract_subgraph_names(schema: &'a SchemaDocument) -> HashMap<String, String> {
        let mut map = HashMap::new();
        let join_graph_enum = schema.definitions.iter().find_map(|d| match d {
            input::Definition::TypeDefinition(input::TypeDefinition::Enum(e)) => {
                if e.name == "join__Graph" {
                    Some(e)
                } else {
                    None
                }
            }
            _ => None,
        });

        if let Some(join_graph_enum) = join_graph_enum {
            for enum_value in join_graph_enum.values.iter() {
                let graph_id = enum_value.name.to_string();
                let join_graphs =
                    Self::extract_directives::<JoinGraphDirective>(&enum_value.directives);

                if let Some(join_graph_directive) = join_graphs.first() {
                    map.insert(graph_id, join_graph_directive.name.to_string());
                }
            }
        }

        map
    }

    #[instrument(skip(schema))]
    fn build_map(schema: &'a SchemaDocument) -> HashMap<String, SupergraphDefinition<'a>> {
        schema
            .definitions
            .iter()
            .filter_map(|definition| match definition {
                input::Definition::TypeDefinition(input::TypeDefinition::Object(object_type)) => {
                    Some((
                        object_type.name.to_string(),
                        SupergraphDefinition::Object(Self::build_object_type(object_type, schema)),
                    ))
                }
                input::Definition::TypeDefinition(input::TypeDefinition::Interface(
                    interface_type,
                )) => Some((
                    interface_type.name.to_string(),
                    SupergraphDefinition::Interface(Self::build_interface_type(interface_type)),
                )),
                input::Definition::TypeDefinition(input::TypeDefinition::Enum(enum_type)) => {
                    Some((
                        enum_type.name.to_string(),
                        SupergraphDefinition::Enum(Self::build_enum_type(enum_type)),
                    ))
                }
                input::Definition::TypeDefinition(input::TypeDefinition::Union(union_type)) => {
                    Some((
                        union_type.name.to_string(),
                        SupergraphDefinition::Union(Self::build_union_type(union_type)),
                    ))
                }
                input::Definition::TypeDefinition(input::TypeDefinition::Scalar(scalar_type)) => {
                    Some((
                        scalar_type.name.to_string(),
                        SupergraphDefinition::Scalar(Self::build_scalar_type(scalar_type)),
                    ))
                }
                input::Definition::TypeDefinition(input::TypeDefinition::InputObject(
                    input_object_type,
                )) => Some((
                    input_object_type.name.to_string(),
                    SupergraphDefinition::InputObject(Self::build_input_object_type(
                        input_object_type,
                    )),
                )),
                _ => None,
            })
            .collect()
    }

    #[instrument(skip(input_object_type), fields(name = input_object_type.name))]
    fn build_input_object_type(
        input_object_type: &'a input::InputObjectType<'static, String>,
    ) -> SupergraphInputObjectType<'a> {
        SupergraphInputObjectType {
            source: input_object_type,
            join_type: Self::extract_directives::<JoinTypeDirective>(&input_object_type.directives),
        }
    }

    #[instrument(skip(scalar_type), fields(name = scalar_type.name))]
    fn build_scalar_type(
        scalar_type: &'a input::ScalarType<'static, String>,
    ) -> SupergraphScalarType<'a> {
        SupergraphScalarType {
            source: scalar_type,
            join_type: Self::extract_directives::<JoinTypeDirective>(&scalar_type.directives),
        }
    }

    #[instrument(skip(union_type), fields(name = union_type.name))]
    fn build_union_type(
        union_type: &'a input::UnionType<'static, String>,
    ) -> SupergraphUnionType<'a> {
        SupergraphUnionType {
            source: union_type,
            join_type: Self::extract_directives::<JoinTypeDirective>(&union_type.directives),
            types: union_type.types.clone(),
            union_members: Self::extract_directives::<JoinUnionMemberDirective>(
                &union_type.directives,
            ),
        }
    }

    #[instrument(skip(enum_type), fields(name = enum_type.name))]
    fn build_enum_type(enum_type: &'a input::EnumType<'static, String>) -> SupergraphEnumType<'a> {
        SupergraphEnumType {
            source: enum_type,
            join_type: Self::extract_directives::<JoinTypeDirective>(&enum_type.directives),
            values: enum_type
                .values
                .iter()
                .map(|value| SupergraphEnumValueType {
                    source: value,
                    join_enum_value: Self::extract_directives::<JoinEnumValueDirective>(
                        &value.directives,
                    ),
                })
                .collect(),
        }
    }
    #[instrument(skip(fields), fields(fields_count = fields.len()))]
    fn build_fields(
        fields: &'a [input::Field<'static, String>],
    ) -> HashMap<String, SupergraphField<'a>> {
        fields
            .iter()
            .map(|field| {
                (
                    field.name.to_string(),
                    SupergraphField {
                        source: field,
                        join_field: Self::extract_directives::<JoinFieldDirective>(
                            &field.directives,
                        ),
                        inaccessible: !Self::extract_directives::<InaccessibleDirective>(
                            &field.directives,
                        )
                        .is_empty(),
                    },
                )
            })
            .collect()
    }

    #[instrument(skip(interface_type), fields(name = interface_type.name))]
    fn build_interface_type(
        interface_type: &'a input::InterfaceType<'static, String>,
    ) -> SupergraphInterfaceType<'a> {
        let fields = Self::build_fields(&interface_type.fields);
        let used_in_subgraphs = Self::build_subgraph_usage_from_fields(&fields);

        SupergraphInterfaceType {
            source: interface_type,
            fields,
            join_type: Self::extract_directives::<JoinTypeDirective>(&interface_type.directives),
            join_implements: Self::extract_directives::<JoinImplementsDirective>(
                &interface_type.directives,
            ),
            used_in_subgraphs,
        }
    }

    #[instrument(skip(object_type, schema), fields(name = object_type.name))]
    fn build_object_type(
        object_type: &'a input::ObjectType<'static, String>,
        schema: &'a SchemaDocument,
    ) -> SupergraphObjectType<'a> {
        let fields = Self::build_fields(&object_type.fields);

        let root_type = if object_type.name == schema.query_type().name {
            Some(RootOperationType::Query)
        } else if schema
            .mutation_type()
            .is_some_and(|t| t.name == object_type.name)
        {
            Some(RootOperationType::Mutation)
        } else if schema
            .subscription_type()
            .is_some_and(|t| t.name == object_type.name)
        {
            Some(RootOperationType::Subscription)
        } else {
            None
        };

        let used_in_subgraphs = Self::build_subgraph_usage_from_fields(&fields);

        SupergraphObjectType {
            source: object_type,
            fields,
            join_type: Self::extract_directives::<JoinTypeDirective>(&object_type.directives),
            join_implements: Self::extract_directives::<JoinImplementsDirective>(
                &object_type.directives,
            ),
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
                if let Some(graph) = &join_field.graph_id {
                    subgraphs.insert(graph.to_string());
                }
            }
        }

        subgraphs
    }

    fn extract_directives<D: FederationDirective<'a>>(
        directives: &[Directive<'static, String>],
    ) -> Vec<D> {
        directives
            .iter()
            .filter_map(|directive| {
                if D::is(directive) {
                    Some(D::parse(directive))
                } else {
                    None
                }
            })
            .collect()
    }
}

#[derive(Debug, Clone, Copy)]
pub enum RootOperationType {
    Query,
    Mutation,
    Subscription,
}

impl Display for RootOperationType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RootOperationType::Query => write!(f, "Query"),
            RootOperationType::Mutation => write!(f, "Mutation"),
            RootOperationType::Subscription => write!(f, "Subscription"),
        }
    }
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
pub struct SupergraphObjectType<'a> {
    pub source: &'a input::ObjectType<'static, String>,
    pub fields: HashMap<String, SupergraphField<'a>>,
    pub join_type: Vec<JoinTypeDirective>,
    pub join_implements: Vec<JoinImplementsDirective>,
    pub root_type: Option<RootOperationType>,
    pub used_in_subgraphs: HashSet<String>,
}

impl SupergraphObjectType<'_> {
    pub fn fields_of_subgraph(
        &self,
        graph_id: &str,
    ) -> HashMap<&String, (&SupergraphField<'_>, Option<JoinFieldDirective>)> {
        self.fields
            .iter()
            .filter_map(|(_field_name, field_def)| {
                let no_join_field = field_def.join_field.is_empty();

                let current_graph_graph_jf = field_def
                    .join_field
                    .iter()
                    // TODO: handle override: "something"
                    .find(|jf| jf.graph_id.as_ref().is_some_and(|g| g == graph_id));

                if no_join_field || current_graph_graph_jf.is_some() {
                    Some((_field_name, (field_def, current_graph_graph_jf.cloned())))
                } else {
                    None
                }
            })
            .collect()
    }
}

impl SupergraphInterfaceType<'_> {
    pub fn fields_of_subgraph(
        &self,
        graph_id: &str,
    ) -> HashMap<&String, (&SupergraphField<'_>, Option<JoinFieldDirective>)> {
        self.fields
            .iter()
            .filter_map(|(_field_name, field_def)| {
                let no_join_field = field_def.join_field.is_empty();

                let current_graph_graph_jf = field_def
                    .join_field
                    .iter()
                    .find(|jf| jf.graph_id.as_ref().is_some_and(|g| g == graph_id));

                if no_join_field || current_graph_graph_jf.is_some() {
                    Some((_field_name, (field_def, current_graph_graph_jf.cloned())))
                } else {
                    None
                }
            })
            .collect()
    }
}

#[derive(Debug)]
pub struct SupergraphInterfaceType<'a> {
    pub source: &'a input::InterfaceType<'static, String>,
    pub fields: HashMap<String, SupergraphField<'a>>,
    pub join_type: Vec<JoinTypeDirective>,
    pub join_implements: Vec<JoinImplementsDirective>,
    pub used_in_subgraphs: HashSet<String>,
}

#[derive(Debug)]
pub struct SupergraphEnumValueType<'a> {
    pub source: &'a input::EnumValue<'static, String>,
    pub join_enum_value: Vec<JoinEnumValueDirective>,
}

#[derive(Debug)]
pub struct SupergraphInputObjectType<'a> {
    pub source: &'a input::InputObjectType<'static, String>,
    pub join_type: Vec<JoinTypeDirective>,
}

#[derive(Debug)]
pub struct SupergraphScalarType<'a> {
    pub source: &'a input::ScalarType<'static, String>,
    pub join_type: Vec<JoinTypeDirective>,
}

#[derive(Debug)]
pub struct SupergraphEnumType<'a> {
    pub source: &'a input::EnumType<'static, String>,
    pub values: Vec<SupergraphEnumValueType<'a>>,
    pub join_type: Vec<JoinTypeDirective>,
}

impl SupergraphEnumType<'_> {
    pub fn values_of_subgraph(&self, graph_id: &str) -> Vec<&SupergraphEnumValueType<'_>> {
        self.values
            .iter()
            .filter(|value| value.join_enum_value.iter().any(|je| je.graph == graph_id))
            .collect::<Vec<_>>()
    }
}

#[derive(Debug)]
pub struct SupergraphUnionType<'a> {
    pub source: &'a input::UnionType<'static, String>,
    pub types: Vec<String>,
    pub join_type: Vec<JoinTypeDirective>,
    pub union_members: Vec<JoinUnionMemberDirective>,
}

impl SupergraphUnionType<'_> {
    pub fn relevant_types(&self, graph_id: &str) -> HashSet<&String> {
        self.union_members
            .iter()
            .filter_map(|um| {
                if um.graph == graph_id {
                    Some(&um.member)
                } else {
                    None
                }
            })
            .collect()
    }
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

    pub fn extract_join_types_for(&self, graph_id: &str) -> Vec<JoinTypeDirective> {
        self.join_types()
            .iter()
            .filter(|jt| jt.graph_id == graph_id)
            .cloned()
            .collect()
    }

    pub fn is_defined_in_subgraph(&self, graph_id: &str) -> bool {
        !self.extract_join_types_for(graph_id).is_empty()
    }

    pub fn is_interface(&self) -> bool {
        matches!(self, SupergraphDefinition::Interface(_))
    }

    pub fn is_root(&self) -> bool {
        match self {
            SupergraphDefinition::Object(object_type) => object_type.root_type.is_some(),
            _ => false,
        }
    }

    pub fn try_into_root_type(&self) -> Option<&RootOperationType> {
        match self {
            SupergraphDefinition::Object(object_type) => object_type.root_type.as_ref(),
            _ => None,
        }
    }

    pub fn fields(&self) -> &HashMap<String, SupergraphField> {
        static EMPTY: std::sync::LazyLock<HashMap<String, SupergraphField>> =
            std::sync::LazyLock::new(HashMap::<String, SupergraphField>::new);

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
            SupergraphDefinition::Union(union_type) => &union_type.join_type,
            SupergraphDefinition::Enum(enum_type) => &enum_type.join_type,
            SupergraphDefinition::Scalar(scalar_type) => &scalar_type.join_type,
            SupergraphDefinition::InputObject(input_object_type) => &input_object_type.join_type,
        }
    }

    pub fn subgraphs(&self) -> HashSet<&str> {
        self.join_types()
            .iter()
            .map(|join_type| join_type.graph_id.as_str())
            .collect::<HashSet<&str>>()
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
}

#[derive(Debug)]
pub struct SupergraphField<'a> {
    pub source: &'a input::Field<'static, String>,
    pub inaccessible: bool,
    pub join_field: Vec<JoinFieldDirective>,
}
