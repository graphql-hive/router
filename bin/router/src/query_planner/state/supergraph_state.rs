use ahash::{HashMap, HashMapExt, HashSet, HashSetExt};
use std::{
    collections::BTreeSet,
    fmt::{Debug, Display},
    sync::Arc,
};

use graphql_tools::parser::query::Directive;
use graphql_tools::parser::schema as input;
use serde::{Deserialize, Serialize};
use tracing::instrument;

use crate::query_planner::{
    federation_spec::{
        demand_control::{CostDirective, ListSizeDirective},
        directives::{
            AuthenticatedDirective, FederationDirective, InaccessibleDirective,
            JoinEnumValueDirective, JoinFieldDirective, JoinGraphDirective,
            JoinImplementsDirective, JoinTypeDirective, JoinUnionMemberDirective, PolicyDirective,
            RequiresScopesDirective,
        },
    },
    graph::edge::{OverrideLabel, Percentage},
};

use super::{
    entity_keys::EntityKeys,
    subgraph_state::{SubgraphDefinitionSource, SubgraphState},
};

static BUILTIN_SCALARS: [&str; 5] = ["String", "Int", "Float", "Boolean", "ID"];

pub type SchemaDocument = input::Document<'static, String>;

#[derive(Debug, thiserror::Error, Clone)]
pub enum SupergraphStateError {
    #[error("Subgraph not found: '{0}'")]
    SubgraphNotFound(String),
    #[error("Root type not found for operation kind: '{0}'")]
    RootTypeNotFound(OperationKind),
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
    /// Std set, the request pipeline keeps the labels it resolves in one.
    pub flags: std::collections::HashSet<String>,
    /// A set of all percentage values used in `@override(label:)`
    pub percentages: HashSet<Percentage>,
}

type InterfaceObjectToSubgraphsMap = HashMap<String, HashSet<String>>;
type InterfaceToObjectTypesMap = HashMap<String, BTreeSet<String>>;
type AbstractPossibleTypesMap = HashMap<String, HashSet<String>>;
type AbstractPossibleTypesBySubgraphMap = HashMap<String, AbstractPossibleTypesMap>;
type DefinitionMap = HashMap<String, SupergraphDefinition>;

/// Information about linked specifications used in the supergraph
/// (e.g., authenticated, requiresScopes, policy)
struct LinkedSpecifications {
    pub authenticated: bool,
    pub requires_scopes: bool,
    pub policy: bool,
}

impl LinkedSpecifications {
    fn from_schema(schema: &SchemaDocument) -> Self {
        let Some(schema_def) = schema.definitions.iter().find_map(|def| match def {
            input::Definition::SchemaDefinition(schema_def) => Some(schema_def),
            _ => None,
        }) else {
            return Self {
                authenticated: false,
                requires_scopes: false,
                policy: false,
            };
        };

        let mut authenticated = false;
        let mut requires_scopes = false;
        let mut policy = false;

        for directive in &schema_def.directives {
            // Found all of them? Stop searching.
            if authenticated && requires_scopes && policy {
                break;
            }

            if directive.name != "link" {
                continue;
            }

            let url = directive.arguments.iter().find_map(|(name, value)| {
                if name == "url" {
                    if let graphql_tools::parser::query::Value::String(s) = value {
                        return Some(s);
                    }
                }
                None
            });

            let Some(url) = url else {
                continue;
            };

            if !authenticated && url.starts_with("https://specs.apollo.dev/authenticated/") {
                authenticated = true;
            } else if !requires_scopes
                && url.starts_with("https://specs.apollo.dev/requiresScopes/")
            {
                requires_scopes = true;
            } else if !policy && url.starts_with("https://specs.apollo.dev/policy/") {
                policy = true;
            }
        }

        Self {
            authenticated,
            requires_scopes,
            policy,
        }
    }

    /// Conditionally extract @authenticated directives based on whether the spec is enabled
    fn extract_authenticated_directives(
        &self,
        directives: &[Directive<'static, String>],
    ) -> Vec<AuthenticatedDirective> {
        if self.authenticated {
            SupergraphState::extract_directives::<AuthenticatedDirective>(directives)
        } else {
            Default::default()
        }
    }

    /// Conditionally extract @requiresScopes directives based on whether the spec is enabled
    fn extract_requires_scopes_directives(
        &self,
        directives: &[Directive<'static, String>],
    ) -> Vec<RequiresScopesDirective> {
        if self.requires_scopes {
            SupergraphState::extract_directives::<RequiresScopesDirective>(directives)
        } else {
            Default::default()
        }
    }

    /// Conditionally extract @policy directives based on whether the spec is enabled
    fn extract_policy_directives(
        &self,
        directives: &[Directive<'static, String>],
    ) -> Vec<PolicyDirective> {
        if self.policy {
            SupergraphState::extract_directives::<PolicyDirective>(directives)
        } else {
            Default::default()
        }
    }

    fn extract_cost_directive(
        &self,
        directives: &[Directive<'static, String>],
    ) -> Option<CostDirective> {
        SupergraphState::extract_directives::<CostDirective>(directives)
            .into_iter()
            .next()
    }

    fn extract_list_size_directive(
        &self,
        directives: &[Directive<'static, String>],
    ) -> Option<ListSizeDirective> {
        SupergraphState::extract_directives::<ListSizeDirective>(directives)
            .into_iter()
            .next()
    }
}

#[derive(Debug)]
pub struct SupergraphState {
    /// A map all of definitions (def_name, def) that exists in the schema.
    pub definitions: DefinitionMap,
    /// A map of (SUBGRAPH_ID, subgraph_name) to make it easy to resolve
    pub known_subgraphs: HashMap<String, String>,
    /// Subgraph names, sorted. A subgraph's index is its position here.
    pub subgraph_names: Vec<String>,
    /// A set of all known scalars in this schema, including built-ins
    pub known_scalars: HashSet<String>,
    /// A map from subgraph name to a subgraph state
    pub subgraphs_state: HashMap<SubgraphName, SubgraphState>,
    /// A map of (subgraph_name, endpoint) to make it easy to resolve
    /// Std map, the executor builds its subgraph clients from it.
    pub subgraph_endpoint_map: std::collections::HashMap<String, String>,
    /// The root entrypoints
    pub query_type: String,
    pub mutation_type: Option<String>,
    pub subscription_type: Option<String>,
    /// Holds a map of interface names to a set of subgraph ids
    /// that hold the @interfaceObject
    pub interface_object_types_in_subgraphs: InterfaceObjectToSubgraphsMap,
    /// Holds a map of interface names to all object types implementing them.
    pub interface_to_object_types: InterfaceToObjectTypesMap,
    /// Abstract types (and objects) mapped to their possible concrete types across the supergraph.
    abstract_possible_types: AbstractPossibleTypesMap,
    /// Abstract types (and objects) mapped to their possible concrete types by graph ID.
    abstract_possible_types_by_subgraph: AbstractPossibleTypesBySubgraphMap,
    /// A pre-computed set of all progressive override labels in the supergraph
    pub progressive_overrides: ProgressiveOverrides,
    /// Each subgraph's resolvable `@key`s, by entity type, parsed once. The graph's entity moves
    /// use them, and the executor's `SchemaMetadata` shares them.
    pub entity_keys: Arc<EntityKeys>,
}

impl SupergraphState {
    #[instrument(level = "trace", skip(schema), name = "new_supergraph_state")]
    pub fn new(schema: &SchemaDocument) -> Self {
        let (known_subgraphs, subgraph_endpoint_map) =
            Self::extract_subgraph_names_and_endpoints(schema);
        let linked_specs = LinkedSpecifications::from_schema(schema);

        let mut subgraph_names: Vec<String> = known_subgraphs.values().cloned().collect();
        subgraph_names.sort();

        let mut definitions = Self::build_map(schema, linked_specs);
        Self::fill_resolvable_in(&mut definitions, &known_subgraphs, &subgraph_names);

        let interface_object_types_in_subgraphs =
            Self::create_interface_object_in_subgraph(&definitions);

        let interface_to_object_types = Self::create_interface_to_object_types(&definitions);

        let (abstract_possible_types, abstract_possible_types_by_subgraph) =
            Self::create_abstract_possible_types(&definitions, &interface_to_object_types);

        let progressive_overrides = Self::extract_progressive_overrides(&definitions);

        let known_scalars = Self::extract_known_scalars(schema);
        let query_type = schema.query_type().name.to_string();
        let mutation_type = schema.mutation_type().map(|t| t.name.to_string());
        let subscription_type = schema.subscription_type().map(|t| t.name.to_string());

        let mut instance = Self {
            definitions,
            interface_object_types_in_subgraphs,
            interface_to_object_types,
            abstract_possible_types,
            abstract_possible_types_by_subgraph,
            progressive_overrides,
            known_subgraphs,
            subgraph_names,
            subgraph_endpoint_map,
            known_scalars,
            subgraphs_state: HashMap::new(),
            query_type,
            mutation_type,
            subscription_type,
            entity_keys: Arc::default(),
        };

        // Group the join types by subgraph in one pass. Scanning every definition for every
        // subgraph is too slow when there are hundreds of subgraphs.
        let mut definitions_by_subgraph: HashMap<&str, Vec<SubgraphDefinitionSource>> =
            HashMap::new();
        for (def_name, definition) in instance.definitions.iter() {
            for join_type in definition.join_types() {
                let sources = definitions_by_subgraph
                    .entry(join_type.graph_id.as_str())
                    .or_default();
                // A type can repeat @join__type for one subgraph, keep those together.
                match sources.last_mut() {
                    Some((name, _, join_types)) if *name == def_name => {
                        join_types.push(join_type.clone())
                    }
                    _ => sources.push((def_name, definition, vec![join_type.clone()])),
                }
            }
        }

        for subgraph_id in instance.known_subgraphs.keys() {
            let sources = definitions_by_subgraph
                .remove(subgraph_id.as_str())
                .unwrap_or_default();
            let state = SubgraphState::decompose_from_supergraph(subgraph_id, sources);
            let subgraph_name = instance.resolve_graph_id(subgraph_id).unwrap();
            instance.subgraphs_state.insert(subgraph_name, state);
        }

        instance.entity_keys = Arc::new(EntityKeys::new(&instance));

        instance
    }

    pub fn maybe_root_type(&self, type_name: &str) -> Option<OperationKind> {
        if self.query_type == type_name {
            Some(OperationKind::Query)
        } else if self.mutation_type.as_deref() == Some(type_name) {
            Some(OperationKind::Mutation)
        } else if self.subscription_type.as_deref() == Some(type_name) {
            Some(OperationKind::Subscription)
        } else {
            None
        }
    }

    /// The subgraph's position in `subgraph_names`.
    pub fn subgraph_index(&self, subgraph_name: &str) -> Option<u16> {
        index_in(&self.subgraph_names, subgraph_name)
    }

    /// Which subgraphs resolve each field, worked out once, so planning doesn't rebuild it for
    /// every field it walks.
    fn fill_resolvable_in(
        definitions: &mut DefinitionMap,
        known_subgraphs: &HashMap<String, String>,
        subgraph_names: &[String],
    ) {
        let index_of = |graph_id: &str| {
            known_subgraphs
                .get(graph_id)
                .and_then(|name| index_in(subgraph_names, name))
        };

        for definition in definitions.values_mut() {
            let (join_types, fields) = match definition {
                SupergraphDefinition::Object(object) => (&object.join_type, &mut object.fields),
                SupergraphDefinition::Interface(interface) => {
                    (&interface.join_type, &mut interface.fields)
                }
                _ => continue,
            };

            for field in fields.values_mut() {
                let mut indices: Vec<u16> = if field.join_field.is_empty() {
                    // No @join__field: every subgraph that defines the type resolves it.
                    join_types
                        .iter()
                        .filter_map(|join_type| index_of(&join_type.graph_id))
                        .collect()
                } else {
                    // Otherwise the subgraphs where it's neither external nor overridden.
                    field
                        .join_field
                        .iter()
                        .filter(|jf| !jf.external && !jf.used_overridden)
                        .filter(|jf| jf.override_label.is_none())
                        .filter_map(|jf| jf.graph_id.as_deref().and_then(index_of))
                        .collect()
                };
                indices.sort_unstable();
                indices.dedup();
                field.resolvable_in = indices.into_boxed_slice();
            }
        }
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
        if BUILTIN_SCALARS.contains(&type_name) {
            return true;
        }

        self.known_scalars.contains(type_name)
    }

    pub fn is_custom_scalar_type(&self, type_name: &str) -> bool {
        !BUILTIN_SCALARS.contains(&type_name) && self.known_scalars.contains(type_name)
    }

    pub fn is_interface_object_in_subgraph(&self, type_name: &str, graph_id: &str) -> bool {
        self.interface_object_types_in_subgraphs
            .get(type_name)
            .is_some_and(|subgraph_ids| subgraph_ids.contains(graph_id))
    }

    pub fn interface_members(&self, interface_name: &str) -> Option<&BTreeSet<String>> {
        self.interface_to_object_types.get(interface_name)
    }

    /// Possible concrete types for `type_name` across the entire supergraph.
    pub(crate) fn all_possible_types(&self, type_name: &str) -> Option<&HashSet<String>> {
        self.abstract_possible_types.get(type_name)
    }

    /// Possible concrete types for `type_name` in a subgraph, identified by its graph ID.
    pub(crate) fn possible_types_in_subgraph(
        &self,
        type_name: &str,
        graph_id: &str,
    ) -> Option<&HashSet<String>> {
        self.abstract_possible_types_by_subgraph
            .get(graph_id)
            .and_then(|types| types.get(type_name))
    }

    pub fn field_return_type_name(&self, type_name: &str, field_name: &str) -> Option<&str> {
        if field_name == "__typename" {
            return Some("String");
        }

        let definition = self.definitions.get(type_name)?;
        let field_definition = definition.fields().get(field_name)?;
        Some(field_definition.field_type.inner_type())
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

    fn create_interface_to_object_types(definitions: &DefinitionMap) -> InterfaceToObjectTypesMap {
        let mut interface_to_object_types = InterfaceToObjectTypesMap::new();

        for definition in definitions.values() {
            let SupergraphDefinition::Object(object_type) = definition else {
                continue;
            };

            for join_implements in &object_type.join_implements {
                interface_to_object_types
                    .entry(join_implements.interface.clone())
                    .or_default()
                    .insert(object_type.name.clone());
            }
        }

        interface_to_object_types
    }

    fn create_abstract_possible_types(
        definitions: &DefinitionMap,
        interface_to_object_types: &InterfaceToObjectTypesMap,
    ) -> (AbstractPossibleTypesMap, AbstractPossibleTypesBySubgraphMap) {
        let mut all = AbstractPossibleTypesMap::new();
        let mut by_subgraph = AbstractPossibleTypesBySubgraphMap::new();

        // Index each type's global possible types and create graph-local entries. Building the
        // graph-local interface sets in a second pass avoids scanning every object once per
        // interface.
        for (type_name, definition) in definitions {
            match definition {
                SupergraphDefinition::Object(object_type) => {
                    let singleton = HashSet::from_iter([type_name.clone()]);
                    all.insert(type_name.clone(), singleton.clone());

                    for join_type in &object_type.join_type {
                        by_subgraph
                            .entry(join_type.graph_id.clone())
                            .or_default()
                            .insert(type_name.clone(), singleton.clone());
                    }
                }
                SupergraphDefinition::Union(union_type) => {
                    let global_members = union_type
                        .union_members
                        .iter()
                        .map(|member| member.member.clone())
                        .collect();
                    all.insert(type_name.clone(), global_members);

                    for join_type in &union_type.join_type {
                        let members = union_type
                            .union_members
                            .iter()
                            .filter(|member| member.graph == join_type.graph_id)
                            .map(|member| member.member.clone())
                            .collect();
                        by_subgraph
                            .entry(join_type.graph_id.clone())
                            .or_default()
                            .insert(type_name.clone(), members);
                    }
                }
                SupergraphDefinition::Interface(interface_type) => {
                    let implementors = interface_to_object_types
                        .get(type_name)
                        .map(|types| types.iter().cloned().collect())
                        .unwrap_or_default();
                    all.insert(type_name.clone(), implementors);

                    for join_type in &interface_type.join_type {
                        // Keep an empty entry for an interface with no object implementors in this
                        // subgraph, matching normalization's previous distinction between an
                        // empty set and a type absent from the subgraph.
                        by_subgraph
                            .entry(join_type.graph_id.clone())
                            .or_default()
                            .insert(type_name.clone(), HashSet::new());
                    }
                }
                _ => {}
            }
        }

        for (type_name, definition) in definitions {
            let SupergraphDefinition::Object(object_type) = definition else {
                continue;
            };

            for join_implements in &object_type.join_implements {
                let Some(types) = by_subgraph.get_mut(&join_implements.graph_id) else {
                    continue;
                };
                if !types.contains_key(type_name) {
                    continue;
                }
                let Some(implementors) = types.get_mut(&join_implements.interface) else {
                    continue;
                };

                // The object must itself be defined in the graph. Its singleton entry in this
                // graph's map is only created from its @join__type directives.
                implementors.insert(type_name.clone());
            }
        }

        (all, by_subgraph)
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

        for builtin in BUILTIN_SCALARS {
            set.insert(builtin.to_string());
        }

        set
    }

    fn extract_subgraph_names_and_endpoints(
        schema: &SchemaDocument,
    ) -> (
        HashMap<String, String>,
        std::collections::HashMap<String, String>,
    ) {
        let mut subgraph_names_map = HashMap::new();
        let mut subgraph_endpoints_map = std::collections::HashMap::new();
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

    #[instrument(level = "trace", skip_all)]
    fn build_map(
        schema: &SchemaDocument,
        linked_specs: LinkedSpecifications,
    ) -> HashMap<String, SupergraphDefinition> {
        // Each of these scans the whole document, so look them up once, not per object type.
        let query_type = schema.query_type().name.as_str();
        let mutation_type = schema.mutation_type().map(|t| t.name.as_str());
        let subscription_type = schema.subscription_type().map(|t| t.name.as_str());
        let root_type_of = |name: &str| {
            if name == query_type {
                Some(OperationKind::Query)
            } else if mutation_type == Some(name) {
                Some(OperationKind::Mutation)
            } else if subscription_type == Some(name) {
                Some(OperationKind::Subscription)
            } else {
                None
            }
        };

        schema
            .definitions
            .iter()
            .filter_map(|definition| match definition {
                input::Definition::TypeDefinition(input::TypeDefinition::Object(object_type)) => {
                    Some((
                        object_type.name.to_string(),
                        SupergraphDefinition::Object(Self::build_object_type(
                            object_type,
                            root_type_of(&object_type.name),
                            &linked_specs,
                        )),
                    ))
                }
                input::Definition::TypeDefinition(input::TypeDefinition::Interface(
                    interface_type,
                )) => Some((
                    interface_type.name.to_string(),
                    SupergraphDefinition::Interface(Self::build_interface_type(
                        interface_type,
                        &linked_specs,
                    )),
                )),
                input::Definition::TypeDefinition(input::TypeDefinition::Enum(enum_type)) => {
                    Some((
                        enum_type.name.to_string(),
                        SupergraphDefinition::Enum(Self::build_enum_type(enum_type, &linked_specs)),
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
                        SupergraphDefinition::Scalar(Self::build_scalar_type(
                            scalar_type,
                            &linked_specs,
                        )),
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

    #[instrument(level = "trace", skip_all, fields(name = input_object_type.name))]
    fn build_input_object_type(
        input_object_type: &input::InputObjectType<'static, String>,
    ) -> SupergraphInputObjectType {
        SupergraphInputObjectType {
            name: input_object_type.name.to_string(),
            fields: Self::build_input_fields(&input_object_type.fields),
            join_type: Self::extract_directives::<JoinTypeDirective>(&input_object_type.directives),
        }
    }

    #[instrument(level = "trace", skip_all, fields(name = scalar_type.name))]
    fn build_scalar_type(
        scalar_type: &input::ScalarType<'static, String>,
        linked_specs: &LinkedSpecifications,
    ) -> SupergraphScalarType {
        SupergraphScalarType {
            name: scalar_type.name.to_string(),
            join_type: Self::extract_directives::<JoinTypeDirective>(&scalar_type.directives),
            authenticated: linked_specs.extract_authenticated_directives(&scalar_type.directives),
            requires_scopes: linked_specs
                .extract_requires_scopes_directives(&scalar_type.directives),
            policy: linked_specs.extract_policy_directives(&scalar_type.directives),
            cost: linked_specs.extract_cost_directive(&scalar_type.directives),
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

    #[instrument(level = "trace",skip_all, fields(name = enum_type.name))]
    fn build_enum_type(
        enum_type: &input::EnumType<'static, String>,
        linked_specs: &LinkedSpecifications,
    ) -> SupergraphEnumType {
        let values: Vec<SupergraphEnumValueType> = enum_type
            .values
            .iter()
            .map(|value| SupergraphEnumValueType {
                name: value.name.to_string(),
                join_enum_value: Self::extract_directives::<JoinEnumValueDirective>(
                    &value.directives,
                ),
            })
            .collect();

        let mut values_by_subgraph: HashMap<String, Vec<usize>> = HashMap::new();
        for (index, value) in values.iter().enumerate() {
            for join_enum_value in &value.join_enum_value {
                let indexes = values_by_subgraph
                    .entry(join_enum_value.graph.clone())
                    .or_default();
                // A value can list the same subgraph twice.
                if indexes.last() != Some(&index) {
                    indexes.push(index);
                }
            }
        }

        SupergraphEnumType {
            name: enum_type.name.to_string(),
            join_type: Self::extract_directives::<JoinTypeDirective>(&enum_type.directives),
            authenticated: linked_specs.extract_authenticated_directives(&enum_type.directives),
            requires_scopes: linked_specs.extract_requires_scopes_directives(&enum_type.directives),
            policy: linked_specs.extract_policy_directives(&enum_type.directives),
            values,
            values_by_subgraph,
            cost: linked_specs.extract_cost_directive(&enum_type.directives),
        }
    }

    #[instrument(level = "trace",skip_all, fields(fields_count = fields.len()))]
    fn build_fields(
        fields: &[input::Field<'static, String>],
        linked_specs: &LinkedSpecifications,
    ) -> HashMap<String, SupergraphField> {
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
                        authenticated: linked_specs
                            .extract_authenticated_directives(&field.directives),
                        requires_scopes: linked_specs
                            .extract_requires_scopes_directives(&field.directives),
                        policy: linked_specs.extract_policy_directives(&field.directives),
                        inaccessible: !Self::extract_directives::<InaccessibleDirective>(
                            &field.directives,
                        )
                        .is_empty(),
                        cost: linked_specs.extract_cost_directive(&field.directives),
                        list_size: linked_specs.extract_list_size_directive(&field.directives),
                        resolvable_in: Box::default(),
                        cost_by_arguments: field
                            .arguments
                            .iter()
                            .filter_map(|arg| {
                                let cost_directive =
                                    linked_specs.extract_cost_directive(&arg.directives);

                                cost_directive.map(|cost| (arg.name.to_string(), cost))
                            })
                            .collect(),
                        argument_types: field
                            .arguments
                            .iter()
                            .map(|arg| (arg.name.to_string(), (&arg.value_type).into()))
                            .collect(),
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
                        authenticated: Default::default(),
                        requires_scopes: Default::default(),
                        policy: Default::default(),
                        inaccessible: !Self::extract_directives::<InaccessibleDirective>(
                            &field.directives,
                        )
                        .is_empty(),
                        cost: Self::extract_directives::<CostDirective>(&field.directives)
                            .into_iter()
                            .next(),
                        list_size: None,
                        resolvable_in: Box::default(),
                        cost_by_arguments: Default::default(),
                        argument_types: Default::default(),
                    },
                )
            })
            .collect()
    }

    #[instrument(level = "trace",skip_all, fields(name = interface_type.name))]
    fn build_interface_type(
        interface_type: &input::InterfaceType<'static, String>,
        linked_specs: &LinkedSpecifications,
    ) -> SupergraphInterfaceType {
        let fields = Self::build_fields(&interface_type.fields, linked_specs);
        let used_in_subgraphs = Self::build_subgraph_usage_from_fields(&fields);

        SupergraphInterfaceType {
            name: interface_type.name.to_string(),
            fields,
            join_type: Self::extract_directives::<JoinTypeDirective>(&interface_type.directives),
            join_implements: Self::extract_directives::<JoinImplementsDirective>(
                &interface_type.directives,
            ),
            authenticated: linked_specs
                .extract_authenticated_directives(&interface_type.directives),
            requires_scopes: linked_specs
                .extract_requires_scopes_directives(&interface_type.directives),
            policy: linked_specs.extract_policy_directives(&interface_type.directives),
            used_in_subgraphs,
        }
    }

    #[instrument(level = "trace",skip_all, fields(name = object_type.name))]
    fn build_object_type(
        object_type: &input::ObjectType<'static, String>,
        root_type: Option<OperationKind>,
        linked_specs: &LinkedSpecifications,
    ) -> SupergraphObjectType {
        let fields = Self::build_fields(&object_type.fields, linked_specs);
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
            authenticated: linked_specs.extract_authenticated_directives(&object_type.directives),
            requires_scopes: linked_specs
                .extract_requires_scopes_directives(&object_type.directives),
            policy: linked_specs.extract_policy_directives(&object_type.directives),
            cost: linked_specs.extract_cost_directive(&object_type.directives),
        }
    }

    fn build_subgraph_usage_from_fields(
        fields: &HashMap<String, SupergraphField>,
    ) -> HashSet<String> {
        let mut subgraphs = HashSet::new();

        // Add subgraphs from join_field directives
        for field in fields.values() {
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

    pub fn root_type_name(
        &self,
        operation_kind: Option<&OperationKind>,
    ) -> Result<&str, SupergraphStateError> {
        match operation_kind {
            None | Some(OperationKind::Query) => Ok(&self.query_type),
            Some(OperationKind::Mutation) => {
                self.mutation_type
                    .as_deref()
                    .ok_or(SupergraphStateError::RootTypeNotFound(
                        OperationKind::Mutation,
                    ))
            }
            Some(OperationKind::Subscription) => {
                self.subscription_type
                    .as_deref()
                    .ok_or(SupergraphStateError::RootTypeNotFound(
                        OperationKind::Subscription,
                    ))
            }
        }
    }

    /// panics if the root type is not found for the given operation kind.
    /// Use only when you are sure the root type is present,
    /// like after the operation was validated to have matching root type.
    pub fn expect_root_type_name(&self, operation_kind: Option<&OperationKind>) -> &str {
        self.root_type_name(operation_kind)
            // SAFETY: The root type is guaranteed to exist for the given operation kind,
            // as the operation was validated.
            .unwrap_or_else(|err| panic!("{}", err))
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum OperationKind {
    #[serde(rename = "query")]
    Query,
    #[serde(rename = "mutation")]
    Mutation,
    #[serde(rename = "subscription")]
    Subscription,
}

impl OperationKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            OperationKind::Query => "query",
            OperationKind::Mutation => "mutation",
            OperationKind::Subscription => "subscription",
        }
    }

    pub fn is_query(&self) -> bool {
        matches!(self, OperationKind::Query)
    }

    pub fn is_mutation(&self) -> bool {
        matches!(self, OperationKind::Mutation)
    }
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

impl TryFrom<&str> for OperationKind {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "query" => Ok(OperationKind::Query),
            "mutation" => Ok(OperationKind::Mutation),
            "subscription" => Ok(OperationKind::Subscription),
            _ => Err("invalid operation kind".to_string()),
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
    pub requires_scopes: Vec<RequiresScopesDirective>,
    pub policy: Vec<PolicyDirective>,
    pub authenticated: Vec<AuthenticatedDirective>,
    pub cost: Option<CostDirective>,
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
    pub requires_scopes: Vec<RequiresScopesDirective>,
    pub policy: Vec<PolicyDirective>,
    pub authenticated: Vec<AuthenticatedDirective>,
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
    pub requires_scopes: Vec<RequiresScopesDirective>,
    pub policy: Vec<PolicyDirective>,
    pub authenticated: Vec<AuthenticatedDirective>,
    pub cost: Option<CostDirective>,
}

#[derive(Debug)]
pub struct SupergraphEnumType {
    pub name: String,
    pub values: Vec<SupergraphEnumValueType>,
    /// Indexes into `values`, per subgraph, so `values_of_subgraph` doesn't scan them all.
    values_by_subgraph: HashMap<String, Vec<usize>>,
    pub join_type: Vec<JoinTypeDirective>,
    pub requires_scopes: Vec<RequiresScopesDirective>,
    pub policy: Vec<PolicyDirective>,
    pub authenticated: Vec<AuthenticatedDirective>,
    pub cost: Option<CostDirective>,
}

impl SupergraphEnumType {
    pub fn values_of_subgraph(&self, graph_id: &str) -> Vec<&SupergraphEnumValueType> {
        self.values_by_subgraph
            .get(graph_id)
            .map(|indexes| indexes.iter().map(|&index| &self.values[index]).collect())
            .unwrap_or_default()
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
    pub requires_scopes: Vec<RequiresScopesDirective>,
    pub policy: Vec<PolicyDirective>,
    pub authenticated: Vec<AuthenticatedDirective>,
    pub cost: Option<CostDirective>,
    pub list_size: Option<ListSizeDirective>,
    /// The subgraphs that resolve this field, as sorted indices into
    /// `SupergraphState::subgraph_names`.
    pub resolvable_in: Box<[u16]>,
    pub cost_by_arguments: HashMap<String, CostDirective>,
    pub argument_types: HashMap<String, TypeNode>,
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

    pub fn unwrap_non_null(&self) -> &TypeNode {
        match self {
            TypeNode::NonNull(inner) => inner.unwrap_non_null(),
            _ => self,
        }
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

fn index_in(subgraph_names: &[String], subgraph_name: &str) -> Option<u16> {
    subgraph_names
        .binary_search_by(|name| name.as_str().cmp(subgraph_name))
        .ok()
        .map(|index| index as u16)
}

#[cfg(test)]
mod tests {
    use ahash::HashSet;

    use crate::query_planner::utils::parsing::parse_schema;

    use super::SupergraphState;

    #[test]
    fn resolvable_in_skips_external_fields() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/fixture/tests/override_requires.supergraph.graphql"
        );
        let supergraph =
            SupergraphState::new(&parse_schema(&std::fs::read_to_string(path).unwrap()));
        let resolvable_in = |field: &str| -> Vec<&str> {
            supergraph.definitions["User"].fields()[field]
                .resolvable_in
                .iter()
                .map(|index| supergraph.subgraph_names[*index as usize].as_str())
                .collect()
        };

        // No @join__field, so wherever `User` is.
        assert_eq!(resolvable_in("id"), ["a", "b", "c"]);
        // External in `a` and `c`.
        assert_eq!(resolvable_in("name"), ["b"]);
        assert_eq!(resolvable_in("aName"), ["a"]);
    }

    #[test]
    fn precomputes_possible_abstract_types_globally_and_per_subgraph() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/fixture/tests/partial-union-complex.supergraph.graphql"
        );
        let supergraph =
            SupergraphState::new(&parse_schema(&std::fs::read_to_string(path).unwrap()));

        let expected_all = HashSet::from_iter([
            "Common".to_string(),
            "OnlyA".to_string(),
            "OnlyB".to_string(),
        ]);
        assert_eq!(supergraph.all_possible_types("Action"), Some(&expected_all));
        assert_eq!(
            supergraph.possible_types_in_subgraph("Action", "A"),
            Some(&HashSet::from_iter([
                "Common".to_string(),
                "OnlyA".to_string()
            ]))
        );
        assert_eq!(
            supergraph.possible_types_in_subgraph("Action", "B"),
            Some(&HashSet::from_iter([
                "Common".to_string(),
                "OnlyB".to_string()
            ]))
        );

        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/fixture/tests/abstract-types.supergraph.graphql"
        );
        let supergraph =
            SupergraphState::new(&parse_schema(&std::fs::read_to_string(path).unwrap()));
        let expected_implementors =
            HashSet::from_iter(["Book".to_string(), "Magazine".to_string()]);
        assert_eq!(
            supergraph.all_possible_types("Product"),
            Some(&expected_implementors)
        );
        assert_eq!(
            supergraph.possible_types_in_subgraph("Product", "PRODUCTS"),
            Some(&expected_implementors)
        );
    }
}
