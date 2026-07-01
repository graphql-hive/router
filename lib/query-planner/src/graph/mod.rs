pub(crate) mod edge;
pub(crate) mod error;
pub(crate) mod node;

pub use self::edge::PlannerOverrideContext;
pub use self::edge::PERCENTAGE_SCALE_FACTOR;

mod tests;

use std::sync::Arc;
use std::{
    collections::{HashMap, HashSet},
    fmt::{Debug, Display},
};

use super::ast::normalization::utils::extract_type_condition;
use crate::state::supergraph_state::SubgraphName;
use crate::{
    federation_spec::{CachedFederationRules, FederationRules},
    graph::node::{SubgraphTypeSpecialization, UnionMembersData},
    state::supergraph_state::{
        OperationKind, SupergraphDefinition, SupergraphField, SupergraphState,
    },
};
use ahash::AHashMap;
use error::GraphError;
use graphql_tools::parser::query::{Selection, SelectionSet};
use petgraph::{
    dot::Dot,
    graph::{EdgeIndex, Edges, NodeIndex},
    visit::EdgeRef,
    Directed, Direction, Graph as Petgraph,
};
use tracing::{instrument, trace};

use super::graph::{edge::Edge, node::Node};

type InnerGraph<'graph> = Petgraph<Node<'graph>, Edge<'graph>, Directed>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct SubgraphNodeKey<'graph> {
    type_name: &'graph str,
    graph_id: &'graph str,
    is_interface_object: bool,
}

#[derive(Debug, Default)]
struct GraphBuildContext<'graph> {
    resolved_graph_ids: HashMap<&'graph str, SubgraphName<'graph>>,
    #[allow(dead_code)]
    subgraph_nodes: HashMap<SubgraphNodeKey<'graph>, NodeIndex>,
}

impl<'graph> GraphBuildContext<'graph> {
    fn resolve_graph_id(
        &mut self,
        state: &'graph SupergraphState,
        graph_id: &'graph str,
    ) -> Result<SubgraphName<'graph>, GraphError> {
        if !self.resolved_graph_ids.contains_key(graph_id) {
            self.resolved_graph_ids
                .insert(graph_id, state.resolve_graph_id(graph_id)?);
        }

        Ok(*self
            .resolved_graph_ids
            .get(graph_id)
            .expect("graph id should be cached"))
    }
}

type UnionTypeName<'a> = &'a str;
type SubgraphNameStr<'a> = &'a str;
type UnionMemberTypes<'a> = HashSet<&'a str>;
type UnionRegistyHashMap<'a> =
    HashMap<UnionTypeName<'a>, HashMap<SubgraphNameStr<'a>, UnionMemberTypes<'a>>>;

#[derive(Debug, Default)]
struct UnionDefinitions<'a> {
    registry: UnionRegistyHashMap<'a>,
}

impl<'a> UnionDefinitions<'a> {
    pub fn new(state: &'a SupergraphState) -> Self {
        let mut registry: UnionRegistyHashMap<'a> = UnionRegistyHashMap::new();

        for (def_name, definition) in state
            .definitions
            .iter()
            .filter(|(_, d)| matches!(d, SupergraphDefinition::Union(_)))
        {
            let mut in_subgraphs: HashMap<SubgraphNameStr<'a>, UnionMemberTypes<'a>> =
                HashMap::new();

            for join_member in definition.join_union_members() {
                in_subgraphs
                    .entry(&join_member.graph)
                    .and_modify(|e| {
                        e.insert(&join_member.member);
                    })
                    .or_insert_with(|| {
                        let mut set: UnionMemberTypes<'a> = HashSet::new();
                        set.insert(&join_member.member);
                        set
                    });
            }

            registry.insert(def_name, in_subgraphs);
        }

        Self { registry }
    }

    /// Checks if a type_name exists in the registry of union type definitions.
    /// Basically a check whether a type is a union.
    pub fn contains(&self, type_name: &'a str) -> bool {
        self.registry.contains_key(type_name)
    }

    fn members_in_subgraph(&self, type_name: &str, graph: &str) -> Option<&UnionMemberTypes<'a>> {
        self.registry.get(type_name).and_then(|r| r.get(graph))
    }

    /// Produces the union members visible from a field resolved in a subgraph
    pub fn members_for_field_in_graph(
        &self,
        field_def: &'a SupergraphField,
        field_type: &str,
        graph_id: &str,
    ) -> UnionMemberTypes<'a> {
        // Collect subgraphs the field was defined in.
        // First, look for join__field(graph:),
        // If not defined, look at type's join__type(graph:).
        if let Some(join_field) = field_def.join_field.iter().find(|join_field| {
            join_field
                .graph_id
                .as_ref()
                .is_some_and(|field_graph_id| field_graph_id == graph_id)
        }) {
            if let Some(type_in_graph) = join_field.type_in_graph.as_ref().map(|t| t.inner_type()) {
                // join__field(type:) can narrow a union-returning field to one concrete member.
                if type_in_graph != field_type {
                    return UnionMemberTypes::from([type_in_graph]);
                }
            }
        }

        self.members_in_subgraph(field_type, graph_id)
            .cloned()
            .unwrap_or_default()
    }
}

#[derive(Debug, Default)]
pub struct Graph<'graph> {
    pub graph: InnerGraph<'graph>,
    pub query_root: NodeIndex,
    pub mutation_root: Option<NodeIndex>,
    pub subscription_root: Option<NodeIndex>,
    node_to_index: HashMap<Node<'graph>, NodeIndex>,
    pub node_display_name_to_index: HashMap<String, NodeIndex>,
}

impl<'graph> Graph<'graph> {
    #[instrument(level = "trace", skip(supergraph_state))]
    pub fn graph_from_supergraph_state(
        supergraph_state: &'graph SupergraphState,
    ) -> Result<Self, GraphError> {
        let mut instance = Graph {
            node_to_index: HashMap::new(),
            node_display_name_to_index: HashMap::new(),
            graph: InnerGraph::new(),
            ..Default::default()
        };

        instance.build_graph(supergraph_state)?;

        Ok(instance)
    }

    fn needs_output_traversal(definition: &SupergraphDefinition) -> bool {
        matches!(
            definition,
            SupergraphDefinition::Object(_)
                | SupergraphDefinition::Interface(_)
                | SupergraphDefinition::Union(_)
        )
    }

    fn can_have_entity_moves(definition: &SupergraphDefinition) -> bool {
        matches!(
            definition,
            SupergraphDefinition::Object(_) | SupergraphDefinition::Interface(_)
        )
    }

    fn is_leaf_output_type(state: &SupergraphState, type_name: &str) -> bool {
        state.is_scalar_type(type_name)
            || state
                .definitions
                .get(type_name)
                .is_some_and(|definition| matches!(definition, SupergraphDefinition::Enum(_)))
    }

    pub fn node(&self, node_index: NodeIndex) -> Result<&Node<'_>, GraphError> {
        self.graph
            .node_weight(node_index)
            .ok_or(GraphError::NodeNotFound(node_index))
    }

    pub fn edge(&self, edge_index: EdgeIndex) -> Result<&Edge<'graph>, GraphError> {
        self.graph
            .edge_weight(edge_index)
            .ok_or(GraphError::EdgeNotFound(edge_index))
    }

    pub fn get_edge_head(&self, edge_index: &EdgeIndex) -> Result<NodeIndex, GraphError> {
        self.graph
            .edge_endpoints(*edge_index)
            .ok_or(GraphError::EdgeNotFound(*edge_index))
            .map(|v| v.0)
    }

    pub fn get_edge_tail(&self, edge_index: &EdgeIndex) -> Result<NodeIndex, GraphError> {
        self.graph
            .edge_endpoints(*edge_index)
            .ok_or(GraphError::EdgeNotFound(*edge_index))
            .map(|v| v.1)
    }

    #[instrument(level = "trace", skip(self, state))]
    fn build_graph(&mut self, state: &'graph SupergraphState) -> Result<(), GraphError> {
        trace!(
            "Building graph for supergraph with {} definitions",
            state.definitions.len()
        );

        let mut build_context = GraphBuildContext::default();
        let mut cached_rules = CachedFederationRules::new(state);

        self.build_root_nodes(state)?;
        self.link_root_edges(state, &mut build_context)?;
        self.build_field_edges(state, &mut build_context, &mut cached_rules)?;
        self.build_interface_implementation_edges(state, &mut build_context)?;
        self.build_entity_reference_edges(state, &mut build_context, &mut cached_rules)?;
        self.build_viewed_field_edges(state, &mut build_context, &mut cached_rules)?;

        self.node_display_name_to_index = self
            .graph
            .node_indices()
            .map(|node_index| (self.graph[node_index].display_name(), node_index))
            .collect();

        Ok(())
    }

    pub fn pretty_print_node(&self, node_index: &NodeIndex) -> String {
        self.node(*node_index).unwrap().display_name()
    }

    pub fn pretty_print_edge(&self, edge_index: EdgeIndex, without_source: bool) -> String {
        let (source, target) = self.graph.edge_endpoints(edge_index).unwrap();
        let from = self.node(source).unwrap();
        let to = self.node(target).unwrap();
        let edge = self.edge(edge_index).unwrap();

        if without_source {
            format!("-({})- {}", edge, to.display_name())
        } else {
            format!("{} -({})- {}", from.display_name(), edge, to.display_name())
        }
    }

    #[instrument(level = "trace", skip(self, state))]
    fn build_root_nodes(&mut self, state: &'graph SupergraphState) -> Result<(), GraphError> {
        self.query_root = self.upsert_node(Node::QueryRoot(state.query_type.as_str()));
        trace!("added root type for queries: {}", state.query_type);
        self.mutation_root = state.mutation_type.as_ref().map(|mutation_type| {
            trace!("added root type for mutations: {}", mutation_type);
            self.upsert_node(Node::MutationRoot(mutation_type.as_str()))
        });
        self.subscription_root = state.subscription_type.as_ref().map(|subscription_type| {
            trace!("added root type for subscriptions: {}", subscription_type);
            self.upsert_node(Node::SubscriptionRoot(subscription_type.as_str()))
        });

        Ok(())
    }

    pub fn upsert_node(&mut self, node: Node<'graph>) -> NodeIndex {
        if let Some(index) = self.node_to_index.get(&node) {
            return *index;
        }

        let display_identifier = node.display_name();
        let index = self.graph.add_node(node);
        self.node_to_index.insert(self.graph[index].clone(), index);
        self.node_display_name_to_index
            .insert(display_identifier, index);

        index
    }

    fn upsert_subgraph_node(
        &mut self,
        build_context: &mut GraphBuildContext<'graph>,
        state: &'graph SupergraphState,
        type_name: &'graph str,
        graph_id: &'graph str,
        is_interface_object: bool,
    ) -> Result<NodeIndex, GraphError> {
        let key = SubgraphNodeKey {
            type_name,
            graph_id,
            is_interface_object,
        };

        if let Some(index) = build_context.subgraph_nodes.get(&key) {
            return Ok(*index);
        }

        let node_index = self.upsert_node(Node::new_node(
            type_name,
            build_context.resolve_graph_id(state, graph_id)?,
            is_interface_object,
        ));
        build_context.subgraph_nodes.insert(key, node_index);

        Ok(node_index)
    }

    pub fn upsert_edge(
        &mut self,
        head: NodeIndex,
        tail: NodeIndex,
        edge: Edge<'graph>,
    ) -> EdgeIndex {
        let existing_edge = self
            .graph
            .edges_connecting(head, tail)
            .find_map(|edge_ref| {
                let edge_weight = edge_ref.weight();

                if edge_weight == &edge {
                    Some(edge_ref.id())
                } else {
                    None
                }
            });

        if let Some(edge) = existing_edge {
            edge
        } else {
            self.graph.add_edge(head, tail, edge)
        }
    }

    fn push_edge(&mut self, head: NodeIndex, tail: NodeIndex, edge: Edge<'graph>) -> EdgeIndex {
        self.graph.add_edge(head, tail, edge)
    }

    #[instrument(level = "trace", skip(self, state, build_context, cached_rules))]
    fn build_entity_reference_edges(
        &mut self,
        state: &'graph SupergraphState,
        build_context: &mut GraphBuildContext<'graph>,
        cached_rules: &mut CachedFederationRules<'graph>,
    ) -> Result<(), GraphError> {
        let mut implementing_objects: AHashMap<
            &'graph str,
            Vec<(&'graph str, &'graph SupergraphDefinition)>,
        > = AHashMap::default();
        for (object_type_name, object_type_definition) in state
            .definitions
            .iter()
            .filter(|(_name, def)| matches!(def, SupergraphDefinition::Object(..)))
        {
            for join_implements in object_type_definition.join_implements() {
                implementing_objects
                    .entry(join_implements.interface.as_str())
                    .or_default()
                    .push((object_type_name.as_str(), object_type_definition));
            }
        }

        for (def_name, definition) in state
            .definitions
            .iter()
            .filter(|(_, definition)| Self::can_have_entity_moves(definition))
        {
            let is_interface = definition.is_interface_type();
            let join_nodes = definition
                .join_types()
                .iter()
                .map(|join_type| {
                    let node = self.upsert_subgraph_node(
                        build_context,
                        state,
                        def_name,
                        join_type.graph_id.as_str(),
                        join_type.is_interface_object,
                    )?;
                    let key_selection = join_type.key.as_ref().and_then(|key| {
                        join_type.resolvable.then(|| {
                            cached_rules.parse_key(
                                join_type.graph_id.as_str(),
                                def_name,
                                key.as_str(),
                            )
                        })
                    });

                    Ok((join_type, node, key_selection))
                })
                .collect::<Result<Vec<_>, GraphError>>()?;

            for (join_type1, head, key_selection1) in &join_nodes {
                // Connects object and interface entities of the same name by @key
                for (join_type2, tail, key_selection2) in &join_nodes {
                    if join_type1.graph_id != join_type2.graph_id {
                        if let (Some(key), Some(key_selection)) =
                            (&join_type2.key, key_selection2.as_ref())
                        {
                            trace!(
                                "Creating entity move edge from '{}/{}' to '{}/{}' via key '{}'",
                                def_name,
                                join_type1.graph_id,
                                def_name,
                                join_type2.graph_id,
                                key
                            );

                            self.upsert_edge(
                                *head,
                                *tail,
                                Edge::create_entity_move(
                                    key.as_str(),
                                    key_selection.clone(),
                                    is_interface,
                                ),
                            );
                        }
                    } else if let (Some(key), Some(key_selection)) =
                        (&join_type1.key, key_selection1.as_ref())
                    {
                        trace!(
                            "Creating self-referencing entity move edge in '{}/{}' via key '{}'",
                            def_name,
                            join_type1.graph_id,
                            key
                        );

                        self.upsert_edge(
                            *head,
                            *head,
                            Edge::create_entity_move(
                                key.as_str(),
                                key_selection.clone(),
                                is_interface,
                            ),
                        );
                    }
                }

                // Connects object types implementing @interfaceObject by @key
                if !join_type1.is_interface_object {
                    continue;
                }

                // Ignore if the @key is not resolable
                if !join_type1.resolvable {
                    continue;
                }

                // Ignore if there is no @key
                if join_type1.key.is_none() {
                    continue;
                }

                let interface_object_name = def_name;
                let tail = self.upsert_subgraph_node(
                    build_context,
                    state,
                    interface_object_name,
                    &join_type1.graph_id,
                    join_type1.is_interface_object,
                )?;

                let typename_selection = cached_rules.parse_key(
                    join_type1.graph_id.as_str(),
                    interface_object_name,
                    "__typename",
                );

                for &(object_type_name, object_type_definition) in implementing_objects
                    .get(interface_object_name.as_str())
                    .map(Vec::as_slice)
                    .unwrap_or(&[])
                {
                    // In order to support fragments with type conditions
                    // or `__typename` on @interfaceObject
                    // we need tell the Query Planner that this action occured,
                    // so it knows to look for `__typename`,
                    // but using a resolable path.
                    // The subgraph defining the @interfaceObject has no idea,
                    // that it's an interface and what object types implement it.
                    // We need to collect `__typename` remotely (via entity call).
                    trace!(
                        "Creating @interfaceObject to type '{}' move edge from '{}/{}' to '{}/{}' via key '{}'",
                        object_type_name,
                        interface_object_name,
                        join_type1.graph_id,
                        interface_object_name,
                        join_type1.graph_id,
                        "__typename"
                    );
                    self.upsert_edge(
                        tail,
                        tail,
                        Edge::create_interface_object_type_move(
                            object_type_name,
                            typename_selection.clone(),
                        ),
                    );

                    // Connect them via @key of the @interfaceObject.
                    // Safe to expect a key, because of the if statement before.
                    let key = join_type1
                        .key
                        .as_ref()
                        .expect("@interfaceObject to have a key");

                    let key_selection = cached_rules.parse_key(
                        join_type1.graph_id.as_str(),
                        interface_object_name,
                        key.as_str(),
                    );

                    for join_type2 in object_type_definition.join_types() {
                        if join_type1.graph_id == join_type2.graph_id {
                            // it shouldn't really happen as the @interfaceObject is an object type,
                            // so no object types within the same subgraph can implement it,
                            // as it's not an interface.
                            continue;
                        }

                        let head = self.upsert_subgraph_node(
                            build_context,
                            state,
                            object_type_name,
                            &join_type2.graph_id,
                            join_type2.is_interface_object,
                        )?;

                        trace!(
                            "Creating entity move edge from '{}/{}' to '{}/{}' via key '{}'",
                            interface_object_name,
                            join_type1.graph_id,
                            object_type_name,
                            join_type2.graph_id,
                            key
                        );

                        self.upsert_edge(
                            head,
                            tail,
                            Edge::create_entity_move(
                                key.as_str(),
                                key_selection.clone(),
                                is_interface,
                            ),
                        );
                    }
                }
            }
        }

        Ok(())
    }

    #[instrument(level = "trace", skip(self, state, build_context))]
    fn build_interface_implementation_edges(
        &mut self,
        state: &'graph SupergraphState,
        build_context: &mut GraphBuildContext<'graph>,
    ) -> Result<(), GraphError> {
        for (def_name, definition) in state
            .definitions
            .iter()
            .filter(|(_, d)| matches!(d, SupergraphDefinition::Object(_)))
        {
            for join_implements in definition.join_implements() {
                let tail = self.upsert_subgraph_node(
                    build_context,
                    state,
                    def_name,
                    &join_implements.graph_id,
                    false,
                )?;
                let head = self.upsert_subgraph_node(
                    build_context,
                    state,
                    &join_implements.interface,
                    &join_implements.graph_id,
                    false,
                )?;

                trace!(
                    "Building interface implementation edge from '{}/{}' to '{}/{}'",
                    def_name,
                    join_implements.graph_id,
                    join_implements.interface,
                    join_implements.graph_id
                );

                self.push_edge(
                    head,
                    tail,
                    Edge::AbstractMove(definition.name().to_string()),
                );
            }
        }

        Ok(())
    }

    pub fn root_query_node(&self) -> &Node<'_> {
        &self.graph[self.query_root]
    }

    pub fn root_mutation_node(&self) -> Option<&Node<'_>> {
        if let Some(mutation_root) = self.mutation_root {
            Some(&self.graph[mutation_root])
        } else {
            None
        }
    }

    pub fn root_subscription_node(&self) -> Option<&Node<'_>> {
        if let Some(subscription_root) = self.subscription_root {
            Some(&self.graph[subscription_root])
        } else {
            None
        }
    }

    pub fn edges_to(&self, node_index: NodeIndex) -> Edges<'_, Edge<'graph>, Directed> {
        self.graph.edges_directed(node_index, Direction::Incoming)
    }

    pub fn edges_from(&self, node_index: NodeIndex) -> Edges<'_, Edge<'graph>, Directed> {
        self.graph.edges_directed(node_index, Direction::Outgoing)
    }

    #[instrument(level = "trace", skip(self, state, build_context))]
    fn link_root_edges(
        &mut self,
        state: &'graph SupergraphState,
        build_context: &mut GraphBuildContext<'graph>,
    ) -> Result<(), GraphError> {
        for (def_name, definition) in state
            .definitions
            .iter()
            .filter(|(_, definition)| Self::needs_output_traversal(definition))
        {
            if let Some(root_type) = definition.try_into_root_type() {
                for join_type in definition.join_types().iter() {
                    let graph_id = join_type.graph_id.as_str();
                    let relevant_fields = definition
                        .fields()
                        .iter()
                        .filter_map(|(field_name, field_definition)| {
                            let (is_available, _) =
                                FederationRules::check_field_subgraph_availability(
                                    field_definition,
                                    graph_id,
                                    definition,
                                );

                            if is_available {
                                Some(field_name.to_string())
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<_>>();

                    if !relevant_fields.is_empty() {
                        let head = match root_type {
                            OperationKind::Query => Some(self.query_root),
                            OperationKind::Mutation => self.mutation_root,
                            OperationKind::Subscription => self.subscription_root,
                        }
                        .ok_or(GraphError::MissingRootType(root_type.clone()))?;

                        let tail = self.upsert_subgraph_node(
                            build_context,
                            state,
                            def_name,
                            graph_id,
                            state.is_interface_object_in_subgraph(def_name, graph_id),
                        )?;

                        self.push_edge(
                            head,
                            tail,
                            Edge::SubgraphEntrypoint {
                                field_names: relevant_fields,
                                name: build_context.resolve_graph_id(state, graph_id)?,
                            },
                        );
                    }
                }
            }
        }

        Ok(())
    }

    #[instrument(level = "trace", skip(self, state, build_context, cached_rules))]
    fn build_field_edges(
        &mut self,
        state: &'graph SupergraphState,
        build_context: &mut GraphBuildContext<'graph>,
        cached_rules: &mut CachedFederationRules<'graph>,
    ) -> Result<(), GraphError> {
        let unions = UnionDefinitions::new(state);

        for (def_name, definition) in state
            .definitions
            .iter()
            .filter(|(_, definition)| Self::can_have_entity_moves(definition))
        {
            for join_type in definition.join_types().iter() {
                let graph_id = join_type.graph_id.as_str();
                let graph_name = build_context.resolve_graph_id(state, graph_id)?;

                let is_interface_object = join_type.is_interface_object;
                let has_resolvable_typename = matches!(
                    definition,
                    SupergraphDefinition::Object(_)
                        | SupergraphDefinition::Union(_)
                        | SupergraphDefinition::Interface(_)
                ) && !is_interface_object;

                if has_resolvable_typename {
                    let field_name = "__typename";
                    trace!(
                        "[x] Creating owned field move edge '{}.__typename/{}' (type: String)",
                        def_name,
                        graph_id
                    );
                    let head =
                        self.upsert_subgraph_node(build_context, state, def_name, graph_id, false)?;
                    let tail =
                        self.upsert_subgraph_node(build_context, state, "String", graph_id, false)?;

                    self.push_edge(
                        head,
                        tail,
                        Edge::create_field_move(
                            field_name, def_name, true, false, None, None, None,
                        ),
                    );
                }

                trace!(
                    "[x] Creating self-referencing edge for '{}/{}'",
                    def_name,
                    graph_id
                );
                let head = self.upsert_subgraph_node(
                    build_context,
                    state,
                    def_name,
                    graph_id,
                    state.is_interface_object_in_subgraph(def_name, graph_id),
                )?;
                self.push_edge(head, head, Edge::Selfie(def_name.clone()));

                for (field_name, field_definition) in definition.fields().iter() {
                    let (is_available, maybe_join_field) =
                        FederationRules::check_field_subgraph_availability(
                            field_definition,
                            graph_id,
                            definition,
                        );

                    let target_type = field_definition.field_type.inner_type();

                    if !is_available {
                        // The field is not available in the current subgraph
                        trace!(
                              "[ ] Field '{}.{}/{}' is not available in the subgraph, skipping edge creation (type: {})",
                              def_name, field_name, graph_id, target_type
                          );
                        continue;
                    }

                    // A field is considered "overridden" if its resolution is handled by a different subgraph.
                    // This prevents the current subgraph from creating a resolvable edge for a field it no longer owns.
                    let overridden_by = field_definition.join_field.iter().find_map(|jf| {
                        if let Some(override_from) = &jf.override_value {
                            if override_from == &graph_name.0 {
                                let overriding_subgraph_name = state
                                    .resolve_graph_id(jf.graph_id.as_ref().expect(
                                        "@override must be on a @join__field with a graph argument",
                                    ))
                                    .unwrap();
                                return Some((
                                    overriding_subgraph_name.0.to_string(),
                                    jf.override_label.clone(),
                                ));
                            }
                        }
                        None
                    });

                    let is_external = maybe_join_field.is_some_and(|join_field| {
                        join_field.external && join_field.requires.is_none()
                    });

                    if is_external {
                        trace!(
                            "[ ] Field '{}.{}/{}' is external, skipping edge creation",
                            def_name,
                            field_name,
                            graph_id
                        );

                        continue;
                    }

                    let requirements = maybe_join_field.and_then(|join_field| {
                        join_field.requires.as_ref().map(|requires_str| {
                          (requires_str, join_field.graph_id.as_ref().expect("join__field(graph:) should exist when join__field(requires:) exists"))
                        })
                    }).map(|(requires_str, graph_id)| {
                        cached_rules.parse_requires(
                            graph_id.as_str(),
                            def_name,
                            requires_str.as_str(),
                        )
                    });

                    // If a field points to a union type:
                    //
                    // ```
                    //
                    // type Viewer @join__type(graph: A) @join__type(graph: B) {
                    //   media: ViewerMedia
                    //   aMedia: ViewerMedia @join__field(graph: A)
                    //   bMedia: ViewerMedia @join__field(graph: B)
                    //   book: ViewerMedia @join__field(graph: A, type: "Book") @join__field(graph: B, type: "ViewerMedia")
                    //   song: ViewerMedia @join__field(graph: A)
                    // }
                    //
                    // union ViewerMedia
                    //   @join__type(graph: A)
                    //   @join__type(graph: B)
                    //   @join__unionMember(graph: A, member: "Book")
                    //   @join__unionMember(graph: B, member: "Book")
                    //   @join__unionMember(graph: A, member: "Song")
                    //   @join__unionMember(graph: B, member: "Movie") =
                    //   | Book
                    //   | Song
                    //   | Movie
                    //
                    // ```
                    //
                    // Viewer.media  (A,B)   = Book            (product of the intersection of A and B)
                    // Viewer.aMedia (A)     = Book | Song     (no intersection - it lives in a single subgraph)
                    // Viewer.bMedia (A)     = Book | Movie    (no intersection - it lives in a single subgraph)
                    // Viewer.book   (A,B)   = Book            (product of the intersection of A and B)
                    // Viewer.song   (A)     = Book | Sing     (no intersection - it lives in a single subgraph)
                    //
                    // We need to point it to a subset of object types.
                    // We do it by creating one specialized tail carrying member set,
                    // and from the tail we create abstract-move edges to the object types.
                    //
                    let target_type_is_union = unions.contains(target_type);
                    if target_type_is_union {
                        let head = self.upsert_subgraph_node(
                            build_context,
                            state,
                            def_name,
                            graph_id,
                            state.is_interface_object_in_subgraph(def_name, graph_id),
                        )?;

                        // Build union-member edges for the current subgraph only. Doing a global
                        // intersection here strips valid members from pinned paths such as
                        // Query.getResponse -> Response/A -> actions, where A knows the full union.
                        let mut member_types = unions
                            .members_for_field_in_graph(field_definition, target_type, graph_id)
                            .into_iter()
                            .collect::<Vec<_>>();
                        member_types.sort_unstable();

                        if member_types.is_empty() {
                            continue;
                        }

                        let possible_members = Arc::new(member_types.clone());
                        let representative_member = *possible_members
                            .first()
                            .expect("union member list should not be empty");

                        trace!(
                            "Handling a field {}.{}/{} resolving a union type {}",
                            def_name,
                            field_name,
                            graph_id,
                            target_type
                        );

                        let tail = self.upsert_node(Node::new_specialized_node(
                            target_type,
                            build_context.resolve_graph_id(state, graph_id)?,
                            state.is_interface_object_in_subgraph(target_type, graph_id),
                            SubgraphTypeSpecialization::UnionMembers(UnionMembersData {
                                type_name: def_name.as_str(),
                                field_name: field_name.as_str(),
                                object_type_name: representative_member,
                                possible_members: possible_members.clone(),
                                provides: None,
                            }),
                        ));
                        let typename_tail = self.upsert_subgraph_node(
                            build_context,
                            state,
                            "String",
                            graph_id,
                            false,
                        )?;

                        trace!(
                            "  [x] Creating field move edge '{}.__typename/{}' (type: String)",
                            def_name,
                            graph_id
                        );
                        self.push_edge(
                            tail,
                            typename_tail,
                            Edge::create_field_move(
                                "__typename",
                                target_type,
                                true,
                                false,
                                None,
                                None,
                                None,
                            ),
                        );

                        trace!(
                            "  [x] Creating field move edge '{}.{}/{}' (type: String)",
                            def_name,
                            field_name,
                            graph_id
                        );
                        self.push_edge(
                            head,
                            tail,
                            Edge::create_field_move(
                                field_name,
                                def_name,
                                state.is_scalar_type(target_type),
                                field_definition.field_type.is_list(),
                                None,
                                requirements.clone(),
                                overridden_by.clone(),
                            ),
                        );

                        for member in member_types.iter() {
                            let abstract_tail = self.upsert_subgraph_node(
                                build_context,
                                state,
                                member,
                                graph_id,
                                state.is_interface_object_in_subgraph(member, graph_id),
                            )?;

                            trace!(
                                "  [x] Creating abstract move edge for '{}.{}/{}' (union member: {})",
                                def_name, field_name, graph_id, member
                            );
                            self.push_edge(
                                tail,
                                abstract_tail,
                                Edge::AbstractMove(member.to_string()),
                            );
                        }

                        continue;
                    }

                    trace!(
                        "[x] Creating field move edge '{}.{}/{}' (type: {})",
                        def_name,
                        field_name,
                        graph_id,
                        target_type
                    );

                    let is_leaf = Self::is_leaf_output_type(state, target_type);
                    let head = self.upsert_subgraph_node(
                        build_context,
                        state,
                        def_name,
                        graph_id,
                        state.is_interface_object_in_subgraph(def_name, graph_id),
                    )?;
                    let tail = self.upsert_subgraph_node(
                        build_context,
                        state,
                        target_type,
                        graph_id,
                        state.is_interface_object_in_subgraph(target_type, graph_id),
                    )?;

                    trace!(
                        "[x] Creating field move edge '{}.{}/{}' (type: {})",
                        def_name,
                        field_name,
                        graph_id,
                        target_type
                    );

                    self.upsert_edge(
                        head,
                        tail,
                        Edge::create_field_move(
                            field_name,
                            def_name,
                            is_leaf,
                            field_definition.field_type.is_list(),
                            maybe_join_field.map(|join_field| match join_field.provides {
                                Some(_) => {
                                    // This is done in order to "reset" the provided field info, we can probably
                                    // do this in a better way, and extract info from the JoinFieldDirective into the edges, instead of depending on
                                    // the raw directive info.
                                    // TODO: @dotan, can you explain it?
                                    let mut new = join_field.clone();
                                    new.provides = None;
                                    new
                                }
                                None => join_field.clone(),
                            }),
                            requirements,
                            overridden_by.clone(),
                        ),
                    );
                }
            }
        }

        Ok(())
    }

    #[instrument(level = "trace",skip(self, state, build_context, cached_rules, parent_type_def, head), fields(selection_set, parent_type_name = parent_type_def.name()))]
    fn handle_viewed_selection_set(
        &mut self,
        state: &'graph SupergraphState,
        build_context: &mut GraphBuildContext<'graph>,
        cached_rules: &mut CachedFederationRules<'graph>,
        selection_set: &SelectionSet<'static, String>,
        graph_id: &'graph str,
        parent_type_def: &'graph SupergraphDefinition,
        head: NodeIndex,
        view_id: u64,
    ) -> Result<(), GraphError> {
        for jt in parent_type_def
            .join_types()
            .iter()
            .filter(|jt| jt.resolvable && jt.key.is_some() && jt.graph_id != graph_id)
        {
            let tail = self.upsert_subgraph_node(
                build_context,
                state,
                parent_type_def.name(),
                &jt.graph_id,
                jt.is_interface_object,
            )?;
            let key_selection = cached_rules.parse_key(
                jt.graph_id.as_str(),
                parent_type_def.name(),
                jt.key.as_ref().unwrap().as_str(),
            );
            trace!(
                "Creating entity move edge from '{}/{}' to '{}/{}' via key '{}'",
                parent_type_def.name(),
                graph_id,
                parent_type_def.name(),
                jt.graph_id,
                jt.key.as_ref().unwrap()
            );
            self.upsert_edge(
                head,
                tail,
                Edge::create_entity_move(
                    jt.key.as_ref().unwrap(),
                    key_selection,
                    parent_type_def.is_interface_type(),
                ),
            );
        }

        for selection in selection_set.items.iter() {
            match selection {
                Selection::Field(field) => {
                    let is_leaf = field.selection_set.items.is_empty();
                    let field_in_parent =
                        parent_type_def.fields().get(&field.name).ok_or_else(|| {
                            GraphError::FieldDefinitionNotFound(
                                field.name.clone(),
                                parent_type_def.name().to_string(),
                            )
                        })?;
                    let return_type_name = field_in_parent.field_type.inner_type();

                    trace!(
                        "Upserting graph viewed node for '{}.{}'",
                        return_type_name,
                        graph_id,
                    );

                    let tail = self.upsert_node(Node::new_specialized_node(
                        return_type_name,
                        build_context.resolve_graph_id(state, graph_id)?,
                        state.is_interface_object_in_subgraph(return_type_name, graph_id),
                        SubgraphTypeSpecialization::Provides(view_id),
                    ));

                    self.push_edge(tail, tail, Edge::Selfie(return_type_name.to_string()));

                    trace!(
                        "Creating viewed (#{}) field edge for '{}.{}' (type: {})",
                        view_id,
                        parent_type_def.name(),
                        field.name,
                        return_type_name
                    );

                    self.push_edge(
                        head,
                        tail,
                        Edge::create_field_move(
                            field_in_parent.name.as_str(),
                            parent_type_def.name(),
                            state.is_scalar_type(parent_type_def.name()),
                            field_in_parent.field_type.is_list(),
                            None,
                            None,
                            None,
                        ),
                    );

                    if !is_leaf {
                        let return_type =
                            state.definitions.get(return_type_name).ok_or_else(|| {
                                GraphError::DefinitionNotFound(return_type_name.to_string())
                            })?;

                        self.handle_viewed_selection_set(
                            state,
                            build_context,
                            cached_rules,
                            &field.selection_set,
                            graph_id,
                            return_type,
                            tail,
                            view_id,
                        )?;
                    }
                }
                Selection::InlineFragment(fragment) => {
                    let type_name_from_cond = extract_type_condition(
                        fragment.type_condition.as_ref().unwrap_or_else(|| {
                            // Inline fragments without type condition should have been normalized and converted into selection set
                            panic!("Inline fragment without type condition detected");
                        }),
                    );
                    let type_def_from_cond =
                        state.definitions.get(type_name_from_cond).ok_or_else(|| {
                            GraphError::DefinitionNotFound(type_name_from_cond.to_string())
                        })?;
                    let type_name_from_cond = type_def_from_cond.name();

                    // head is either an interface or a union
                    // tail is a type from a type condition (it's an object type - after normalization)
                    let tail = self.upsert_node(Node::new_specialized_node(
                        type_name_from_cond,
                        build_context.resolve_graph_id(state, graph_id)?,
                        state.is_interface_object_in_subgraph(type_name_from_cond, graph_id),
                        SubgraphTypeSpecialization::Provides(view_id),
                    ));

                    self.push_edge(tail, tail, Edge::Selfie(type_name_from_cond.to_string()));

                    // because it's abstract -> object move, add an abstract move edge
                    self.push_edge(
                        head,
                        tail,
                        Edge::AbstractMove(type_name_from_cond.to_string()),
                    );

                    // use object type (tail) when handling selection sets
                    self.handle_viewed_selection_set(
                        state,
                        build_context,
                        cached_rules,
                        &fragment.selection_set,
                        graph_id,
                        type_def_from_cond,
                        tail,
                        view_id,
                    )?;
                }
                Selection::FragmentSpread(_) => {
                    // Fragment spreads should have been normalized (converted into inline fragments) at this point
                    panic!(
                        "Fragment spread detected. Expected either a Field or an Inline Fragment"
                    )
                }
            };
        }

        Ok(())
    }

    #[instrument(level = "trace", skip(self, state, build_context, cached_rules))]
    fn build_viewed_field_edges(
        &mut self,
        state: &'graph SupergraphState,
        build_context: &mut GraphBuildContext<'graph>,
        cached_rules: &mut CachedFederationRules<'graph>,
    ) -> Result<(), GraphError> {
        for (def_name, definition) in state.definitions.iter() {
            for join_type in definition.join_types().iter() {
                let mut view_id = 0;

                // A map of provided types to graph ids that
                // we need to create edges to their matching entity types.
                let mut connection_to_build: HashMap<NodeIndex, String> = HashMap::new();

                for (field_name, field_definition) in definition.fields().iter() {
                    for join_field in field_definition.join_field.iter() {
                        if join_field
                            .graph_id
                            .as_ref()
                            .is_some_and(|v| v == &join_type.graph_id)
                            && join_field.provides.is_some()
                        {
                            if let Some(provides) = &join_field.provides {
                                let selection_set = cached_rules.parse_provides(
                                    join_type.graph_id.as_str(),
                                    field_definition.field_type.inner_type(),
                                    provides.as_str(),
                                );
                                view_id += 1;

                                let head = self.upsert_subgraph_node(
                                    build_context,
                                    state,
                                    definition.name(),
                                    &join_type.graph_id,
                                    state.is_interface_object_in_subgraph(
                                        definition.name(),
                                        &join_type.graph_id,
                                    ),
                                )?;

                                connection_to_build.insert(head, join_type.graph_id.clone());

                                let return_type_name = field_definition.field_type.inner_type();

                                let tail = self.upsert_node(Node::new_specialized_node(
                                    return_type_name,
                                    build_context
                                        .resolve_graph_id(state, &join_type.graph_id)?
                                        .clone(),
                                    state.is_interface_object_in_subgraph(
                                        return_type_name,
                                        &join_type.graph_id,
                                    ),
                                    SubgraphTypeSpecialization::Provides(view_id),
                                ));

                                self.push_edge(
                                    tail,
                                    tail,
                                    Edge::Selfie(return_type_name.to_string()),
                                );

                                trace!(
                                    "Creating viewed (#{}) link for provided field '{}.{}/{:?}' (type: {})",
                                    view_id, def_name, field_name, join_type.graph_id, return_type_name
                                );

                                let requirements =
                                    join_field.requires.as_ref().map(|requires_str| {
                                        cached_rules.parse_requires(
                                            join_field.graph_id.as_ref().unwrap().as_str(),
                                            def_name,
                                            requires_str.as_str(),
                                        )
                                    });

                                self.push_edge(
                                    head,
                                    tail,
                                    Edge::create_field_move(
                                        field_name,
                                        def_name,
                                        state.is_scalar_type(
                                            field_definition.field_type.inner_type(),
                                        ),
                                        field_definition.field_type.is_list(),
                                        Some(join_field.clone()),
                                        requirements,
                                        None,
                                    ),
                                );

                                let return_type =
                                    state.definitions.get(return_type_name).ok_or_else(|| {
                                        GraphError::DefinitionNotFound(return_type_name.to_string())
                                    })?;

                                self.handle_viewed_selection_set(
                                    state,
                                    build_context,
                                    cached_rules,
                                    selection_set.as_ref(),
                                    &join_type.graph_id,
                                    return_type,
                                    tail,
                                    view_id,
                                )?;
                            }
                        }
                    }
                }

                for (head, from_graph_id) in connection_to_build {
                    for jt in definition.join_types().iter().filter(|jt| {
                        jt.resolvable && jt.key.is_some() && jt.graph_id != from_graph_id
                    }) {
                        let tail = self.upsert_subgraph_node(
                            build_context,
                            state,
                            def_name,
                            &jt.graph_id,
                            jt.is_interface_object,
                        )?;
                        let key_selection = cached_rules.parse_key(
                            jt.graph_id.as_str(),
                            def_name,
                            jt.key.as_ref().unwrap().as_str(),
                        );
                        trace!(
                            "Creating entity move edge from '{}/{}' to '{}/{}' via key '{}'",
                            def_name,
                            from_graph_id,
                            def_name,
                            jt.graph_id,
                            jt.key.as_ref().unwrap()
                        );
                        self.upsert_edge(
                            head,
                            tail,
                            Edge::create_entity_move(
                                jt.key.as_ref().unwrap().as_str(),
                                key_selection,
                                definition.is_interface_type(),
                            ),
                        );
                    }
                }
            }
        }

        Ok(())
    }
}

/// Print me with `println!("{}", graph);` to see the graph in DOT/digraph format.
impl Display for Graph<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", Dot::with_config(&self.graph, &[]))
    }
}
