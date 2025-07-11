use std::{
    collections::{HashMap, HashSet},
    fmt::{Debug, Display},
};

use graphql_parser::query::Directive;
use graphql_parser::schema as input;
use graphql_tools::ast::SchemaDocumentExtension;
use serde::{Deserialize, Serialize};
use tracing::instrument;

use crate::{
    federation_spec::directives::{
        FederationDirective, InaccessibleDirective, JoinEnumValueDirective, JoinFieldDirective,
        JoinGraphDirective, JoinImplementsDirective, JoinTypeDirective, JoinUnionMemberDirective,
    },
    graph::edge::{OverrideLabel, Percentage},
};

use super::subgraph_state::SubgraphState;

static BUILDIB_SCALARS: [&str; 5] = ["String", "Int", "Float", "Boolean", "ID"];

pub type SchemaDocument = input::Document<'static, String>;

#[derive(Debug, thiserror::Error, Clone)]
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

#[derive(Debug, Default)]
pub struct ProgressiveOverrides {
    /// A set of all custom string labels used in `@override(label:)`
    pub flags: HashSet<String>,
    /// A set of all percentage values used in `@override(label:)`
    pub percentages: HashSet<Percentage>,
}

type InterfaceObjectToSubgraphsMap = HashMap<String, HashSet<String>>;
type DefinitionMap = HashMap<String, SupergraphDefinition>;

#[derive(Debug)]
pub struct SupergraphState {
    /// A map all of definitions (def_name, def) that exists in the schema.
    pub definitions: DefinitionMap,
    /// A map of (SUBGRAPH_ID, subgraph_name) to make it easy to resolve
    pub known_subgraphs: HashMap<String, String>,
    /// A set of all known scalars in this schema, including built-ins
    pub known_scalars: HashSet<String>,
    /// A map from subgraph name to a subgraph state
    pub subgraphs_state: HashMap<SubgraphName, SubgraphState>,
    /// A map of (subgraph_name, endpoint) to make it easy to resolve
    pub subgraph_endpoint_map: HashMap<String, String>,
    /// The root entrypoints
    pub query_type: String,
    pub mutation_type: Option<String>,
    pub subscription_type: Option<String>,
    /// Holds a map of interface names to a set of subgraph ids
    /// that hold the @interfaceObject
    pub interface_object_types_in_subgraphs: InterfaceObjectToSubgraphsMap,
    /// A pre-computed set of all progressive override labels in the supergraph
    pub progressive_overrides: ProgressiveOverrides,
}

impl SupergraphState {
    #[instrument(level = "trace", skip(schema), name = "new_supergraph_state")]
    pub fn new(schema: &SchemaDocument) -> Self {
        let (known_subgraphs, subgraph_endpoint_map) =
            Self::extract_subgraph_names_and_endpoints(schema);
        let definitions = Self::build_map(schema);
        let interface_object_types_in_subgraphs =
            Self::create_interface_object_in_subgraph(&definitions);
        let progressive_overrides = Self::extract_progressive_overrides(&definitions);

        let mut instance = Self {
            definitions,
            interface_object_types_in_subgraphs,
            progressive_overrides,
            known_subgraphs,
            subgraph_endpoint_map,
            known_scalars: Self::extract_known_scalars(schema),
            subgraphs_state: HashMap::new(),
            query_type: schema.query_type().name.to_string(),
            mutation_type: schema.mutation_type().map(|t| t.name.to_string()),
            subscription_type: schema.subscription_type().map(|t| t.name.to_string()),
        };

        for subgraph_id in instance.known_subgraphs.keys() {
            let state = SubgraphState::decompose_from_supergraph(subgraph_id, &instance);
            let subgraph_name = instance.resolve_graph_id(subgraph_id).unwrap();
            instance.subgraphs_state.insert(subgraph_name, state);
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
        subgraph_name: &SubgraphName,
    ) -> Result<&SubgraphState, SupergraphStateError> {
        self.subgraphs_state
            .get(subgraph_name)
            .ok_or_else(|| SupergraphStateError::SubgraphNotFound(subgraph_name.0.to_string()))
    }

    pub fn subgraph_exists_by_name(&self, name: &str) -> bool {
        self.subgraph_endpoint_map.contains_key(name)
    }

    pub fn is_scalar_type(&self, type_name: &str) -> bool {
        if BUILDIB_SCALARS.contains(&type_name) {
            return true;
        }

        self.known_scalars.contains(type_name)
    }

    pub fn is_interface_object_in_subgraph(&self, type_name: &str, graph_id: &str) -> bool {
        self.interface_object_types_in_subgraphs
            .get(type_name)
            .is_some_and(|subgraph_ids| subgraph_ids.contains(graph_id))
    }

    fn create_interface_object_in_subgraph(
        definitions: &DefinitionMap,
    ) -> InterfaceObjectToSubgraphsMap {
        let mut interface_object_types_in_subgraphs = InterfaceObjectToSubgraphsMap::new();

        for (name, definition) in definitions
            .iter()
            .filter(|(_, def)| matches!(def, SupergraphDefinition::Interface(_)))
        {
            for graph_id in definition.join_types().iter().filter_map(|t| {
                if t.is_interface_object {
                    Some(&t.graph_id)
                } else {
                    None
                }
            }) {
                interface_object_types_in_subgraphs
                    .entry(name.to_string())
                    .or_default()
                    .insert(graph_id.to_string());
            }
        }

        interface_object_types_in_subgraphs
    }

    fn extract_progressive_overrides(definitions: &DefinitionMap) -> ProgressiveOverrides {
        let mut overrides = ProgressiveOverrides::default();

        for definition in definitions.values() {
            for field in definition.fields().values() {
                for join_field in &field.join_field {
                    if let Some(label) = &join_field.override_label {
                        match label {
                            OverrideLabel::Custom(flag) => {
                                overrides.flags.insert(flag.clone());
                            }
                            OverrideLabel::Percentage(p) => {
                                overrides.percentages.insert(*p);
                            }
                        }
                    }
                }
            }
        }
        overrides
    }

    fn extract_known_scalars(schema: &SchemaDocument) -> HashSet<String> {
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

    fn extract_subgraph_names_and_endpoints(
        schema: &SchemaDocument,
    ) -> (HashMap<String, String>, HashMap<String, String>) {
        let mut subgraph_names_map = HashMap::new();
        let mut subgraph_endpoints_map = HashMap::new();
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
                    subgraph_names_map.insert(graph_id, join_graph_directive.name.to_string());
                    subgraph_endpoints_map.insert(
                        join_graph_directive.name.to_string(),
                        join_graph_directive.url.to_string(),
                    );
                }
            }
        }

        (subgraph_names_map, subgraph_endpoints_map)
    }

    #[instrument(level = "trace", skip(schema))]
    fn build_map(schema: &SchemaDocument) -> HashMap<String, SupergraphDefinition> {
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

    #[instrument(level = "trace",skip(input_object_type), fields(name = input_object_type.name))]
    fn build_input_object_type(
        input_object_type: &input::InputObjectType<'static, String>,
    ) -> SupergraphInputObjectType {
        SupergraphInputObjectType {
            name: input_object_type.name.to_string(),
            fields: Self::build_input_fields(&input_object_type.fields),
            join_type: Self::extract_directives::<JoinTypeDirective>(&input_object_type.directives),
        }
    }

    #[instrument(level = "trace",skip(scalar_type), fields(name = scalar_type.name))]
    fn build_scalar_type(scalar_type: &input::ScalarType<'static, String>) -> SupergraphScalarType {
        SupergraphScalarType {
            name: scalar_type.name.to_string(),
            join_type: Self::extract_directives::<JoinTypeDirective>(&scalar_type.directives),
        }
    }

    #[instrument(level = "trace",skip(union_type), fields(name = union_type.name))]
    fn build_union_type(union_type: &input::UnionType<'static, String>) -> SupergraphUnionType {
        SupergraphUnionType {
            name: union_type.name.to_string(),
            join_type: Self::extract_directives::<JoinTypeDirective>(&union_type.directives),
            types: union_type.types.clone(),
            union_members: Self::extract_directives::<JoinUnionMemberDirective>(
                &union_type.directives,
            ),
        }
    }

    #[instrument(level = "trace",skip(enum_type), fields(name = enum_type.name))]
    fn build_enum_type(enum_type: &input::EnumType<'static, String>) -> SupergraphEnumType {
        SupergraphEnumType {
            name: enum_type.name.to_string(),
            join_type: Self::extract_directives::<JoinTypeDirective>(&enum_type.directives),
            values: enum_type
                .values
                .iter()
                .map(|value| SupergraphEnumValueType {
                    name: value.name.to_string(),
                    join_enum_value: Self::extract_directives::<JoinEnumValueDirective>(
                        &value.directives,
                    ),
                })
                .collect(),
        }
    }

    #[instrument(level = "trace",skip(fields), fields(fields_count = fields.len()))]
    fn build_fields(fields: &[input::Field<'static, String>]) -> HashMap<String, SupergraphField> {
        fields
            .iter()
            .map(|field| {
                (
                    field.name.to_string(),
                    SupergraphField {
                        name: field.name.to_string(),
                        field_type: (&field.field_type).into(),
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

    #[instrument(level = "trace",skip(fields), fields(fields_count = fields.len()))]
    fn build_input_fields(
        fields: &[input::InputValue<'static, String>],
    ) -> HashMap<String, SupergraphField> {
        fields
            .iter()
            .map(|field| {
                (
                    field.name.to_string(),
                    SupergraphField {
                        name: field.name.to_string(),
                        field_type: (&field.value_type).into(),
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

    #[instrument(level = "trace",skip(interface_type), fields(name = interface_type.name))]
    fn build_interface_type(
        interface_type: &input::InterfaceType<'static, String>,
    ) -> SupergraphInterfaceType {
        let fields = Self::build_fields(&interface_type.fields);
        let used_in_subgraphs = Self::build_subgraph_usage_from_fields(&fields);

        SupergraphInterfaceType {
            name: interface_type.name.to_string(),
            fields,
            join_type: Self::extract_directives::<JoinTypeDirective>(&interface_type.directives),
            join_implements: Self::extract_directives::<JoinImplementsDirective>(
                &interface_type.directives,
            ),
            used_in_subgraphs,
        }
    }

    #[instrument(level = "trace",skip(object_type, schema), fields(name = object_type.name))]
    fn build_object_type(
        object_type: &input::ObjectType<'static, String>,
        schema: &SchemaDocument,
    ) -> SupergraphObjectType {
        let fields = Self::build_fields(&object_type.fields);

        let root_type = if object_type.name == schema.query_type().name {
            Some(OperationKind::Query)
        } else if schema
            .mutation_type()
            .is_some_and(|t| t.name == object_type.name)
        {
            Some(OperationKind::Mutation)
        } else if schema
            .subscription_type()
            .is_some_and(|t| t.name == object_type.name)
        {
            Some(OperationKind::Subscription)
        } else {
            None
        };

        let used_in_subgraphs = Self::build_subgraph_usage_from_fields(&fields);

        SupergraphObjectType {
            name: object_type.name.to_string(),
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

    fn extract_directives<D: FederationDirective>(
        directives: &[Directive<'static, String>],
    ) -> Vec<D> {
        let mut result = directives
            .iter()
            .filter_map(|directive| {
                if D::is(directive) {
                    Some(D::parse(directive))
                } else {
                    None
                }
            })
            .collect::<Vec<D>>();

        result.sort();
        result
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub enum OperationKind {
    #[serde(rename = "query")]
    Query,
    #[serde(rename = "mutation")]
    Mutation,
    #[serde(rename = "subscription")]
    Subscription,
}

impl Display for OperationKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OperationKind::Query => write!(f, "query"),
            OperationKind::Mutation => write!(f, "mutation"),
            OperationKind::Subscription => write!(f, "subscription"),
        }
    }
}

#[derive(Debug)]
pub enum SupergraphDefinition {
    Object(SupergraphObjectType),
    Interface(SupergraphInterfaceType),
    Union(SupergraphUnionType),
    Enum(SupergraphEnumType),
    Scalar(SupergraphScalarType),
    InputObject(SupergraphInputObjectType),
}

#[derive(Debug)]
pub struct SupergraphObjectType {
    pub name: String,
    pub fields: HashMap<String, SupergraphField>,
    pub join_type: Vec<JoinTypeDirective>,
    pub join_implements: Vec<JoinImplementsDirective>,
    pub root_type: Option<OperationKind>,
    pub used_in_subgraphs: HashSet<String>,
}

impl SupergraphObjectType {
    pub fn fields_of_subgraph(
        &self,
        graph_id: &str,
    ) -> HashMap<&String, (&SupergraphField, Option<JoinFieldDirective>)> {
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

impl SupergraphInterfaceType {
    pub fn fields_of_subgraph(
        &self,
        graph_id: &str,
    ) -> HashMap<&String, (&SupergraphField, Option<JoinFieldDirective>)> {
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
pub struct SupergraphInterfaceType {
    pub name: String,
    pub fields: HashMap<String, SupergraphField>,
    pub join_type: Vec<JoinTypeDirective>,
    pub join_implements: Vec<JoinImplementsDirective>,
    pub used_in_subgraphs: HashSet<String>,
}

#[derive(Debug)]
pub struct SupergraphEnumValueType {
    pub name: String,
    pub join_enum_value: Vec<JoinEnumValueDirective>,
}

#[derive(Debug)]
pub struct SupergraphInputObjectType {
    pub name: String,
    pub fields: HashMap<String, SupergraphField>,
    pub join_type: Vec<JoinTypeDirective>,
}

#[derive(Debug)]
pub struct SupergraphScalarType {
    pub name: String,
    pub join_type: Vec<JoinTypeDirective>,
}

#[derive(Debug)]
pub struct SupergraphEnumType {
    pub name: String,
    pub values: Vec<SupergraphEnumValueType>,
    pub join_type: Vec<JoinTypeDirective>,
}

impl SupergraphEnumType {
    pub fn values_of_subgraph(&self, graph_id: &str) -> Vec<&SupergraphEnumValueType> {
        self.values
            .iter()
            .filter(|value| value.join_enum_value.iter().any(|je| je.graph == graph_id))
            .collect::<Vec<_>>()
    }
}

#[derive(Debug)]
pub struct SupergraphUnionType {
    pub name: String,
    pub types: Vec<String>,
    pub join_type: Vec<JoinTypeDirective>,
    pub union_members: Vec<JoinUnionMemberDirective>,
}

impl SupergraphUnionType {
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

impl SupergraphDefinition {
    pub fn name(&self) -> &str {
        match self {
            SupergraphDefinition::Object(object_type) => &object_type.name,
            SupergraphDefinition::Interface(interface_type) => &interface_type.name,
            SupergraphDefinition::Union(union_type) => &union_type.name,
            SupergraphDefinition::Enum(enum_type) => &enum_type.name,
            SupergraphDefinition::Scalar(scalar_type) => &scalar_type.name,
            SupergraphDefinition::InputObject(input_type) => &input_type.name,
        }
    }

    pub fn is_composite_type(&self) -> bool {
        matches!(
            self,
            SupergraphDefinition::Object(_)
                | SupergraphDefinition::Interface(_)
                | SupergraphDefinition::Union(_)
        )
    }

    pub fn is_interface_type(&self) -> bool {
        matches!(self, SupergraphDefinition::Interface(_))
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

    pub fn try_into_root_type(&self) -> Option<&OperationKind> {
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

    pub fn subgraphs(&self) -> Vec<&str> {
        let mut result = self
            .join_types()
            .iter()
            .map(|join_type| join_type.graph_id.as_str())
            .collect::<Vec<&str>>();
        result.sort();
        result
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

    pub fn join_union_members(&self) -> &Vec<JoinUnionMemberDirective> {
        match self {
            SupergraphDefinition::Union(union_type) => &union_type.union_members,
            SupergraphDefinition::Object(_)
            | SupergraphDefinition::Interface(_)
            | SupergraphDefinition::Enum(_)
            | SupergraphDefinition::Scalar(_)
            | SupergraphDefinition::InputObject(_) => {
                static EMPTY: Vec<JoinUnionMemberDirective> = Vec::new();
                &EMPTY
            }
        }
    }
}

#[derive(Debug)]
pub struct SupergraphField {
    pub name: String,
    pub field_type: TypeNode,
    pub inaccessible: bool,
    pub join_field: Vec<JoinFieldDirective>,
}

impl SupergraphField {
    pub fn resolvable_in_graphs(&self, type_def: &SupergraphDefinition) -> HashSet<String> {
        // A field is resolvable in all defining subgraph when it has no @join__field
        if self.join_field.is_empty() {
            return type_def
                .join_types()
                .iter()
                .map(|j| j.graph_id.to_string())
                .collect::<HashSet<_>>();
        }

        // A field is resolvable when it has @join__field and it's not external or overriden
        return self
            .join_field
            .iter()
            .filter_map(|jf| {
                if jf.graph_id.is_some()
                    && !jf.external
                    && !jf.used_overridden
                    && jf.override_label.is_none()
                {
                    Some(jf.graph_id.as_ref().unwrap().to_string())
                } else {
                    None
                }
            })
            .collect::<HashSet<_>>();
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum TypeNode {
    List(Box<TypeNode>),
    NonNull(Box<TypeNode>),
    Named(String),
}

impl TypeNode {
    pub fn is_non_null(&self) -> bool {
        matches!(self, TypeNode::NonNull(_))
    }

    pub fn is_list(&self) -> bool {
        match self {
            TypeNode::List(_) => true,
            TypeNode::NonNull(inner) => inner.as_ref().is_list(),
            TypeNode::Named(_) => false,
        }
    }

    pub fn inner_type(&self) -> &str {
        match self {
            TypeNode::List(inner) => inner.as_ref().inner_type(),
            TypeNode::NonNull(inner) => inner.as_ref().inner_type(),
            TypeNode::Named(name) => name,
        }
    }

    /// Generally based on https://spec.graphql.org/draft/#SameResponseShape() algorithm
    pub fn can_be_merged_with(&self, other: &TypeNode) -> bool {
        match (self, other) {
            (TypeNode::List(left), TypeNode::List(right)) => left.can_be_merged_with(right),
            (TypeNode::NonNull(left), TypeNode::NonNull(right)) => left.can_be_merged_with(right),
            (TypeNode::Named(left), TypeNode::Named(right)) => left == right,
            _ => false,
        }
    }
}

impl Display for TypeNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TypeNode::List(inner) => write!(f, "[{}]", inner),
            TypeNode::NonNull(inner) => write!(f, "{}!", inner),
            TypeNode::Named(name) => write!(f, "{}", name),
        }
    }
}

impl<'a, T: input::Text<'a>> From<&input::Type<'a, T>> for TypeNode {
    fn from(input_type: &input::Type<'a, T>) -> Self {
        match input_type {
            input::Type::ListType(inner) => TypeNode::List(Box::new(inner.as_ref().into())),
            input::Type::NonNullType(inner) => TypeNode::NonNull(Box::new(inner.as_ref().into())),
            input::Type::NamedType(name) => TypeNode::Named(name.as_ref().to_string()),
        }
    }
}

impl TryFrom<&str> for TypeNode {
    type Error = &'static str;

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        // The implementation now assumes the string is pre-trimmed.
        // We add a check for an empty string, which is invalid.
        if s.is_empty() {
            return Err("Input string for type parsing cannot be empty.");
        }

        // 1. Check for the NonNull operator `!` at the end.
        if let Some(inner) = s.strip_suffix('!') {
            // Recursively parse the inner type.
            let inner_type = TypeNode::try_from(inner)?;
            return Ok(TypeNode::NonNull(Box::new(inner_type)));
        }

        // 2. Check for the List operator `[]`.
        if let Some(inner) = s.strip_prefix('[') {
            if let Some(inner_content) = inner.strip_suffix(']') {
                // Recursively parse the content inside the brackets.
                let inner_type = TypeNode::try_from(inner_content)?;
                return Ok(TypeNode::List(Box::new(inner_type)));
            } else {
                return Err("Mismatched brackets in list type");
            }
        }

        // 3. Base Case: Handle the Named type.
        if !s.contains(['[', ']', '!']) {
            Ok(TypeNode::Named(s.to_string()))
        } else {
            Err("Invalid named type format")
        }
    }
}
