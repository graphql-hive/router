pub(crate) mod edge;
pub(crate) mod error;
pub(crate) mod node;
mod provided;

pub use self::edge::PlannerOverrideContext;
pub use self::edge::PERCENTAGE_SCALE_FACTOR;

mod tests;

use ahash::{HashMap, HashMapExt, HashSet, HashSetExt};
use std::{
    collections::{hash_map::Entry, BTreeMap},
    fmt::{Debug, Display},
    sync::Arc,
};

use super::graph::{
    edge::{ContextArgument, Edge, EdgeReference},
    node::Node,
    provided::Provided,
};
use crate::query_planner::planner::walker::utils::get_entrypoints;
use crate::query_planner::{
    ast::{
        selection_item::SelectionItem,
        selection_set::{InlineFragmentSelection, SelectionSet},
        type_aware_selection::TypeAwareSelection,
    },
    federation_spec::{
        demand_control::{CostDirective, ListSizeDirective},
        directives::{
            AuthenticatedDirective, JoinFieldDirective, JoinImplementsDirective, JoinTypeDirective,
            PolicyDirective, RequiresScopesDirective,
        },
        CachedFederationRules, FederationRules,
    },
    graph::node::{SubgraphTypeSpecialization, UnionMembersData},
    state::supergraph_state::{
        OperationKind, SubgraphName as ResolvedSubgraphName, SupergraphDefinition, SupergraphField,
        SupergraphObjectType, SupergraphState, SupergraphStateError, TypeNode,
    },
};
use error::GraphError;
use graphql_tools::parser::query::{
    Selection as QuerySelection, SelectionSet as QuerySelectionSet,
    TypeCondition as QueryTypeCondition,
};
use petgraph::{
    dot::Dot,
    graph::{EdgeIndex, NodeIndex},
    visit::EdgeRef,
    Directed, Direction, Graph as Petgraph,
};
use tracing::{instrument, trace};

type InnerGraph<'graph> = Petgraph<Node<'graph>, Edge<'graph>, Directed>;

type UnionTypeName<'a> = &'a str;
type SubgraphName<'a> = &'a str;
type UnionMemberTypes<'a> = HashSet<&'a str>;
type UnionRegistyHashMap<'a> =
    HashMap<UnionTypeName<'a>, HashMap<SubgraphName<'a>, UnionMemberTypes<'a>>>;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct SubgraphNodeKey<'state> {
    type_name: &'state str,
    graph_id: &'state str,
    is_interface_object: bool,
}

#[derive(Default)]
struct GraphBuildContext<'state> {
    resolved_graph_ids: HashMap<&'state str, ResolvedSubgraphName>,
    subgraph_nodes: HashMap<SubgraphNodeKey<'state>, NodeIndex>,
}

impl<'state> GraphBuildContext<'state> {
    fn resolve_graph_id(
        &mut self,
        state: &SupergraphState,
        graph_id: &'state str,
    ) -> Result<&ResolvedSubgraphName, GraphError> {
        if !self.resolved_graph_ids.contains_key(graph_id) {
            self.resolved_graph_ids
                .insert(graph_id, state.resolve_graph_id(graph_id)?);
        }

        Ok(self
            .resolved_graph_ids
            .get(graph_id)
            .expect("resolved graph ID should be cached"))
    }
}

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
            let mut in_subgraphs: HashMap<SubgraphName<'a>, UnionMemberTypes<'a>> = HashMap::new();

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
                    return UnionMemberTypes::from_iter([type_in_graph]);
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
    /// The edges `upsert_edge` added, by head and tail, only while building. `push_edge`
    /// never adds one like them, so they're the only ones to look through.
    upserted: HashMap<(NodeIndex, NodeIndex), Vec<EdgeIndex>>,
    /// The field and entity moves out of every node, see `EdgeIndexes`.
    edge_indexes: EdgeIndexes,
    /// Whether a field takes an argument from `@fromContext`. Resolving one looks up the path for
    /// the ancestor that sets it, so a search from one spot can't be reused at another. Set
    /// once the edges are indexed, read through `has_context_arguments`.
    has_context_arguments: bool,
    /// Object types the planner can't tell apart, see `interchangeable_types`, by the group
    /// they're in. Only types that share a group with another are here.
    pub(crate) type_groups: HashMap<&'graph str, u32>,
    /// What decides how each object type that could share a group is planned, field by field,
    /// see `interchangeable_for`.
    pub(crate) type_profiles: HashMap<&'graph str, TypeProfile<'graph>>,
    /// What each field's `@requires` needs, by type and field: the fields, when it's only plain
    /// ones like `"price currency"`, or `None`. Only fields with `@requires` are here.
    pub(crate) required_fields: RequiredFields<'graph>,
}

impl<'graph> Graph<'graph> {
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

    #[instrument(level = "trace", skip(supergraph_state))]
    pub fn graph_from_supergraph_state(
        supergraph_state: &'graph SupergraphState,
    ) -> Result<Self, GraphError> {
        let mut instance = Graph {
            node_to_index: HashMap::new(),
            graph: InnerGraph::new(),
            ..Default::default()
        };

        instance.build_graph(supergraph_state)?;
        instance.upserted = HashMap::new();
        instance.index_edges();
        (instance.type_groups, instance.type_profiles) = interchangeable_types(supergraph_state);
        instance.required_fields = required_fields(supergraph_state);

        for node in instance.graph.node_weights_mut() {
            if let Node::SubgraphType(subgraph_type) = node {
                let name = &subgraph_type.subgraph.0;
                subgraph_type.subgraph_index = supergraph_state
                    .subgraph_index(name)
                    .ok_or_else(|| SupergraphStateError::SubgraphNotFound(name.clone()))?;
            }
        }

        Ok(instance)
    }

    /// Whether `a` and `b` are object types the planner can't tell apart, see
    /// `interchangeable_types`.
    pub(crate) fn interchangeable(&self, a: &str, b: &str) -> bool {
        match (self.type_groups.get(a), self.type_groups.get(b)) {
            (Some(a), Some(b)) => a == b,
            _ => false,
        }
    }

    /// Whether the planner plans `fields` on `a` and on `b` the same way, from objects in
    /// `subgraphs`: interchangeable types, or for these fields, types that every subgraph keys
    /// the same way, in the same interfaces and unions where both are, with the same
    /// `@join__field`s for each of these fields, and in each of `subgraphs`. A subgraph only one
    /// of them is in then has none of these fields, and with one key everywhere, no path goes
    /// through it.
    pub(crate) fn interchangeable_for<'f>(
        &self,
        a: &str,
        b: &str,
        mut fields: impl Iterator<Item = &'f str>,
        mut subgraphs: impl Iterator<Item = &'f str>,
    ) -> bool {
        if self.interchangeable(a, b) {
            return true;
        }
        let (Some(a), Some(b)) = (self.type_profiles.get(a), self.type_profiles.get(b)) else {
            return false;
        };
        a.key == b.key
            && a.subgraphs.iter().all(|(subgraph, memberships)| {
                b.subgraphs
                    .get(subgraph)
                    .is_none_or(|theirs| theirs == memberships)
            })
            && subgraphs.all(|subgraph| {
                a.subgraphs.contains_key(subgraph) && b.subgraphs.contains_key(subgraph)
            })
            && fields.all(|field| {
                field == "__typename"
                    || matches!((a.fields.get(field), b.fields.get(field)), (Some(a), Some(b)) if a == b)
            })
    }

    /// Fills `edge_indexes`, keeping the order `edges_from` gives.
    fn index_edges(&mut self) {
        let mut indexes = EdgeIndexes {
            starts: Vec::with_capacity(self.graph.node_count() + 1),
            edges: Vec::with_capacity(self.graph.edge_count()),
        };
        let mut entity_moves = Vec::new();
        let mut has_context_arguments = false;
        for node in self.graph.node_indices() {
            let fields_start = indexes.edges.len();
            for edge in self.edges_from(node) {
                match edge.weight() {
                    Edge::FieldMove(field) => {
                        has_context_arguments |= !field.context_arguments.is_empty();
                        indexes.edges.push((name_hash(field.name), edge.id()))
                    }
                    Edge::ReentryMove(reentry) => {
                        indexes.edges.push((name_hash(&reentry.name), edge.id()))
                    }
                    // they have no name, see `EdgeIndexes::edges`
                    Edge::EntityMove(_) | Edge::InterfaceObjectTypeMove(_) => {
                        entity_moves.push((NO_NAME, edge.id()))
                    }
                    _ => {}
                }
            }
            // stable, so the edges of one field keep their order
            indexes.edges[fields_start..].sort_by_key(|(hash, _)| *hash);
            let entities_start = indexes.edges.len();
            indexes.edges.append(&mut entity_moves);
            indexes
                .starts
                .push((fields_start as u32, entities_start as u32));
        }
        let end = indexes.edges.len() as u32;
        indexes.starts.push((end, end));
        self.edge_indexes = indexes;
        self.has_context_arguments = has_context_arguments;
    }

    /// The field and re-entry moves out of `node` to the field `name`, like going through
    /// `edges_from` for them.
    pub fn field_edges_from<'s, 'n>(
        &'s self,
        node: NodeIndex,
        name: &'n str,
    ) -> impl Iterator<Item = EdgeReference<'s>> + 'n
    where
        's: 'n,
    {
        let hash = name_hash(name);
        let fields = self.edge_indexes.fields(node);
        let first = fields.partition_point(|(other, _)| *other < hash);
        fields[first..]
            .iter()
            .take_while(move |(other, _)| *other == hash)
            .map(|(_, index)| self.edge_reference(*index))
            .filter(move |edge| match edge.weight() {
                Edge::FieldMove(field) => field.name == name,
                Edge::ReentryMove(reentry) => reentry.name == name,
                _ => false,
            })
    }

    /// Whether the field `name` of the abstract type at `node` can be read there for an object of
    /// `type_name`. Not when `type_name` is one of its types in that subgraph but doesn't resolve
    /// the field itself, like when it's `@external` there: the interface has the field, but that
    /// type's objects don't. Anything else, `node` decides alone.
    pub fn field_resolvable_for(&self, node: NodeIndex, type_name: &str, name: &str) -> bool {
        let member = self.edges_from(node).find_map(|edge| match edge.weight() {
            Edge::AbstractMove(member) if member == type_name => Some(edge.target()),
            _ => None,
        });
        member.is_none_or(|member| self.field_edges_from(member, name).next().is_some())
    }

    /// The entity and interface object moves out of `node`, like going through `edges_from` for
    /// them.
    pub fn entity_moves_from(&self, node: NodeIndex) -> impl Iterator<Item = EdgeReference<'_>> {
        self.edge_indexes
            .entity_moves(node)
            .map(|index| self.edge_reference(index))
    }

    /// Whether a field anywhere in the graph takes an argument from `@fromContext`.
    pub(crate) fn has_context_arguments(&self) -> bool {
        self.has_context_arguments
    }

    pub(crate) fn edge_reference(&self, index: EdgeIndex) -> EdgeReference<'_> {
        // `edge_references().nth(..)` goes through every edge before it, petgraph's iterator
        // has no shortcut for `nth`
        EdgeReference::new(index, &self.graph.raw_edges()[index.index()])
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
        let mut cached_rules = CachedFederationRules::default();

        self.build_root_nodes(state)?;
        self.link_root_edges(state, &mut build_context)?;
        let provides = self.build_field_edges(state, &mut build_context, &mut cached_rules)?;
        self.build_interface_implementation_edges(state, &mut build_context)?;
        self.build_entity_reference_edges(state, &mut build_context, &mut cached_rules)?;
        self.build_provides_edges(state, provides)?;

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
            format!("-({})- {}", edge, to)
        } else {
            format!("{} -({})- {}", from, edge, to)
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
        match self.node_to_index.entry(node) {
            Entry::Occupied(entry) => *entry.get(),
            Entry::Vacant(entry) => {
                let index = self.graph.add_node(entry.key().clone());
                *entry.insert(index)
            }
        }
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
        if let Some(&node_index) = build_context.subgraph_nodes.get(&key) {
            return Ok(node_index);
        }

        let subgraph = build_context.resolve_graph_id(state, graph_id)?.clone();
        let node_index = self.upsert_node(Node::new_node(type_name, subgraph, is_interface_object));
        build_context.subgraph_nodes.insert(key, node_index);
        Ok(node_index)
    }

    fn upsert_edge(&mut self, head: NodeIndex, tail: NodeIndex, edge: Edge<'graph>) -> EdgeIndex {
        let edges = self.upserted.entry((head, tail)).or_default();
        if let Some(&existing) = edges.iter().find(|&&index| self.graph[index] == edge) {
            return existing;
        }
        let index = self.graph.add_edge(head, tail, edge);
        edges.push(index);
        index
    }

    /// Add an edge without scanning the graph for an identical edge.
    /// Use only where the caller constructs each logical edge once.
    fn push_edge(&mut self, head: NodeIndex, tail: NodeIndex, edge: Edge<'graph>) -> EdgeIndex {
        self.graph.add_edge(head, tail, edge)
    }

    // __typename is a meta-field: it carries no declared field definition, so unlike
    // other fields it always resolves to a plain "String" move, regardless of parent type.
    fn push_typename_edge(
        &mut self,
        head: NodeIndex,
        tail: NodeIndex,
        parent_type_name: &'graph str,
    ) {
        self.push_edge(
            head,
            tail,
            Edge::create_field_move(
                "__typename",
                parent_type_name,
                true,
                0,
                None,
                None,
                vec![],
                None,
            ),
        );
    }

    #[instrument(level = "trace", skip(self, state, build_context, cached_rules))]
    fn build_entity_reference_edges(
        &mut self,
        state: &'graph SupergraphState,
        build_context: &mut GraphBuildContext<'graph>,
        cached_rules: &mut CachedFederationRules<'graph>,
    ) -> Result<(), GraphError> {
        // A mutation or subscription field runs where its root type is, and only once.
        // A @key on those root types can't be used to jump between subgraphs,
        // it would run the field again in another subgraph through `_entities`.
        let is_mutation_or_subscription = |name: &str| {
            state.mutation_type.as_deref() == Some(name)
                || state.subscription_type.as_deref() == Some(name)
        };
        for (def_name, definition) in state.definitions.iter().filter(|(name, definition)| {
            Self::can_have_entity_moves(definition) && !is_mutation_or_subscription(name)
        }) {
            let is_interface = definition.is_interface_type();
            let join_types = definition.join_types();

            let mut nodes = Vec::with_capacity(join_types.len());
            for join_type in join_types {
                nodes.push(self.upsert_subgraph_node(
                    build_context,
                    state,
                    def_name,
                    &join_type.graph_id,
                    join_type.is_interface_object,
                )?);
            }

            // Only resolvable join types with a @key can be jumped to.
            let mut keys = Vec::with_capacity(join_types.len());
            let mut targets = Vec::new();
            for (index, join_type) in join_types.iter().enumerate() {
                let key = match (join_type.resolvable, &join_type.key) {
                    (true, Some(key)) => {
                        targets.push(index);
                        let key_selection =
                            cached_rules.parse_key(state, &join_type.graph_id, def_name, key);
                        Some((key.as_str(), key_selection))
                    }
                    _ => None,
                };
                keys.push(key);
            }

            for (index1, join_type1) in join_types.iter().enumerate() {
                let head = nodes[index1];

                // Connects object and interface entities of the same name by @key
                for &index2 in &targets {
                    // In the same subgraph it's a self-move by join_type1's own key.
                    let (tail, key) = if join_type1.graph_id != join_types[index2].graph_id {
                        (nodes[index2], &keys[index2])
                    } else {
                        (head, &keys[index1])
                    };
                    let Some((key, key_selection)) = key else {
                        continue;
                    };

                    trace!(
                        "Creating entity move edge from '{}' to '{}' via key '{}'",
                        self.graph[head],
                        self.graph[tail],
                        key
                    );

                    self.upsert_edge(
                        head,
                        tail,
                        Edge::create_entity_move(key, key_selection.clone(), is_interface),
                    );
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
                let tail = head;

                let typename_selection = cached_rules.parse_key(
                    state,
                    &join_type1.graph_id,
                    interface_object_name,
                    "__typename",
                );

                for (object_type_name, object_type_definition) in state
                    .definitions
                    .iter()
                    .filter(|(_name, def)| matches!(def, SupergraphDefinition::Object(..)))
                {
                    let SupergraphDefinition::Object(object_type) = object_type_definition else {
                        panic!("Expected to get an Object type after filtering");
                    };

                    // Ignore if the object type does not implement the matching interface
                    if !object_type
                        .join_implements
                        .iter()
                        .any(|j| &j.interface == interface_object_name)
                    {
                        continue;
                    }

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
                        state,
                        &join_type1.graph_id,
                        interface_object_name,
                        key,
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
                            Edge::create_entity_move(key, key_selection.clone(), is_interface),
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
                    // The definition are object types,
                    // so it can't be @interfaceObject (it'd has be Interface).
                    false,
                )?;
                let head = self.upsert_subgraph_node(
                    build_context,
                    state,
                    &join_implements.interface,
                    &join_implements.graph_id,
                    // The definition are object types,
                    // so it can't be @interfaceObject (it'd has be Interface).
                    false,
                )?;

                trace!(
                    "Building interface implementation edge from '{}/{}' to '{}/{}'",
                    def_name,
                    join_implements.graph_id,
                    join_implements.interface,
                    join_implements.graph_id
                );

                self.upsert_edge(
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

    pub fn edges_to(&self, node_index: NodeIndex) -> impl Iterator<Item = EdgeReference<'_>> {
        self.graph
            .edges_directed(node_index, Direction::Incoming)
            .map(EdgeReference::from)
    }

    pub fn edges_from(&self, node_index: NodeIndex) -> impl Iterator<Item = EdgeReference<'_>> {
        self.graph
            .edges_directed(node_index, Direction::Outgoing)
            .map(EdgeReference::from)
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
                // `subgraphs()` is sorted but may contain duplicate graph IDs if the SDL has
                // duplicate `@join__type`s. A root entrypoint is unique per root and graph.
                let mut graph_ids = definition.subgraphs();
                graph_ids.dedup();

                // Collect the root graphs that have at least one available field in one pass.
                // Root entrypoints only need to know whether each graph can resolve anything;
                // building a list of all available field names for every graph was unnecessary.
                let mut graphs_without_fields: HashSet<_> = graph_ids.iter().copied().collect();
                for field in definition.fields().values() {
                    if field.join_field.is_empty() {
                        // A field without join metadata is available in every graph containing
                        // its parent type.
                        graphs_without_fields.clear();
                        break;
                    }

                    for graph_id in field
                        .join_field
                        .iter()
                        .filter_map(|join_field| join_field.graph_id.as_deref())
                    {
                        graphs_without_fields.remove(graph_id);
                    }

                    if graphs_without_fields.is_empty() {
                        break;
                    }
                }

                for graph_id in graph_ids {
                    if !graphs_without_fields.contains(graph_id) {
                        let head = match root_type {
                            OperationKind::Query => Some(self.query_root),
                            OperationKind::Mutation => self.mutation_root,
                            OperationKind::Subscription => self.subscription_root,
                        }
                        .ok_or(GraphError::MissingRootType(root_type.clone()))?;

                        let subgraph = build_context.resolve_graph_id(state, graph_id)?.clone();
                        let tail =
                            self.upsert_node(Node::new_root_node(def_name, subgraph.clone()));

                        self.push_edge(
                            head,
                            tail,
                            Edge::SubgraphEntrypoint {
                                name: subgraph,
                                operation_kind: root_type.clone(),
                            },
                        );
                    }
                }
            }
        }

        Ok(())
    }

    /// The selection a context argument reads, parsed for each type declaring that context.
    fn context_selections(
        state: &'graph SupergraphState,
        graph_id: &str,
        context: &str,
        selection: &str,
    ) -> Arc<[TypeAwareSelection<'graph>]> {
        let mut selections: Vec<TypeAwareSelection> = state
            .contexts
            .get(context)
            .into_iter()
            .flatten()
            .map(|type_name| {
                FederationRules::parse_context_selection(state, graph_id, type_name, selection)
            })
            .collect();
        // ancestor paths can stay on an interface even when only its
        // implementations declare the context, so add an interface selection too
        for (interface, members) in &state.interface_to_object_types {
            // keep the direct selection if the interface already declares this context
            if selections
                .iter()
                .any(|selection| selection.type_name == *interface)
            {
                continue;
            }
            // keep each implementation behind its type condition so both field
            // fetching and context rewrites only apply to matching runtime objects
            let items = selections
                .iter()
                .filter(|selection| members.contains(selection.type_name))
                .map(|selection| {
                    SelectionItem::InlineFragment(InlineFragmentSelection {
                        type_condition: selection.type_name.to_string(),
                        selections: selection.selection_set.clone(),
                        skip_if: None,
                        include_if: None,
                    })
                })
                .collect::<Vec<_>>();
            if !items.is_empty() {
                selections.push(TypeAwareSelection {
                    type_name: interface,
                    selection_set: SelectionSet { items },
                });
            }
        }
        selections.into()
    }

    /// Returns the `@provides` of each field edge that has one, for `build_provides_edges`.
    #[instrument(level = "trace", skip(self, state, build_context, cached_rules))]
    fn build_field_edges(
        &mut self,
        state: &'graph SupergraphState,
        build_context: &mut GraphBuildContext<'graph>,
        cached_rules: &mut CachedFederationRules<'graph>,
    ) -> Result<HashMap<EdgeIndex, Provided>, GraphError> {
        let unions = UnionDefinitions::new(state);
        let mut provides = HashMap::new();
        // Fields reading the same context selection in a subgraph share its parsed form.
        let mut context_selections: HashMap<(&str, &str, &str), Arc<[TypeAwareSelection]>> =
            HashMap::new();

        for (def_name, definition) in state
            .definitions
            .iter()
            .filter(|(_, definition)| Self::needs_output_traversal(definition))
        {
            // A type can repeat @join__type for one subgraph (one per @key), so visit each
            // subgraph once. Everything below adds each edge once per subgraph, no dedup needed.
            let mut subgraphs: BTreeMap<&str, bool> = BTreeMap::new();
            for join_type in definition.join_types() {
                *subgraphs.entry(join_type.graph_id.as_str()).or_default() |=
                    join_type.is_interface_object;
            }

            // The fields each subgraph has, sorted once per type. Asking every field's
            // @join__field list about every subgraph adds up on a type in hundreds of them.
            // A field without @join__field is in every subgraph that has the type.
            let mut everywhere = Vec::new();
            let mut fields_in: HashMap<&str, Vec<_>> = HashMap::new();
            for (index, (field_name, field_definition)) in definition.fields().iter().enumerate() {
                if field_definition.join_field.is_empty() {
                    everywhere.push((field_name, field_definition, None));
                    continue;
                }
                for join_field in &field_definition.join_field {
                    let Some(graph_id) = join_field.graph_id.as_deref() else {
                        continue;
                    };
                    let fields = fields_in.entry(graph_id).or_default();
                    // only the first @join__field for a subgraph counts
                    if fields.last().is_none_or(|&(last, _)| last != index) {
                        fields.push((index, (field_name, field_definition, Some(join_field))));
                    }
                }
            }

            for (graph_id, is_interface_object) in subgraphs {
                let graph_name = build_context.resolve_graph_id(state, graph_id)?.0.clone();
                let has_resolvable_typename = matches!(
                    definition,
                    SupergraphDefinition::Object(_)
                        | SupergraphDefinition::Union(_)
                        | SupergraphDefinition::Interface(_)
                ) && !is_interface_object;

                if has_resolvable_typename {
                    trace!(
                        "[x] Creating owned field move edge '{}.__typename/{}' (type: String)",
                        def_name,
                        graph_id
                    );
                    let head = self.upsert_subgraph_node(
                        build_context,
                        state,
                        def_name,
                        graph_id,
                        // __typename is not resolable for @interfaceObject so it's not it
                        false,
                    )?;
                    let tail = self.upsert_subgraph_node(
                        build_context,
                        state,
                        "String",
                        graph_id,
                        // String is not an @interfaceObject
                        false,
                    )?;

                    self.push_typename_edge(head, tail, def_name);
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

                let specific = fields_in.get(graph_id).into_iter().flatten();
                for &(field_name, field_definition, maybe_join_field) in
                    everywhere.iter().chain(specific.map(|(_, field)| field))
                {
                    let target_type = field_definition.field_type.inner_type();

                    // A field is considered "overridden" if its resolution is handled by a different subgraph.
                    // This prevents the current subgraph from creating a resolvable edge for a field it no longer owns.
                    let overridden_by = field_definition.join_field.iter().find_map(|jf| {
                        if let Some(override_from) = &jf.override_value {
                            if override_from == &graph_name {
                                let overriding_subgraph_name = state
                                    .resolve_graph_id(jf.graph_id.as_ref().expect(
                                        "@override must be on a @join__field with a graph argument",
                                    ))
                                    .unwrap();
                                return Some((
                                    overriding_subgraph_name.0,
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
                            cached_rules.parse_requires(
                                state,
                                join_field.graph_id.as_ref().expect(
                                    "join__field(graph:) should exist when join__field(requires:) exists",
                                ),
                                def_name,
                                requires_str,
                            )
                        })
                    });

                    let context_arguments = maybe_join_field
                        .map(|join_field| {
                            let graph_id = join_field
                                .graph_id
                                .as_ref()
                                .expect("context arguments must have a subgraph");
                            join_field
                                .context_arguments
                                .iter()
                                .map(|argument| ContextArgument {
                                    name: argument.name.clone(),
                                    type_in_graph: argument.type_in_graph.clone(),
                                    selections: context_selections
                                        .entry((graph_id, &argument.context, &argument.selection))
                                        .or_insert_with(|| {
                                            Self::context_selections(
                                                state,
                                                graph_id,
                                                &argument.context,
                                                &argument.selection,
                                            )
                                        })
                                        .clone(),
                                })
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();

                    // Parsed once, a union field has an edge per member.
                    let provided = maybe_join_field
                        .and_then(|join_field| {
                            let graph_id = join_field.graph_id.as_ref()?;
                            cached_rules.parse_provides(state, join_field, graph_id, target_type)
                        })
                        .map(|selection_set| Provided::from_selection_set(&selection_set));

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
                    // We do it by creating a new Node for each edge's tail,
                    // and from the tail we create abstract-move edges to the object types.
                    //
                    let target_type_is_union = unions.contains(target_type);
                    if target_type_is_union {
                        // Build union-member edges for the current subgraph only. Doing a global
                        // intersection here strips valid members from pinned paths such as
                        // Query.getResponse -> Response/A -> actions, where A knows the full union.
                        let mut member_types = unions
                            .members_for_field_in_graph(field_definition, target_type, graph_id)
                            .into_iter()
                            .collect::<Vec<_>>();
                        member_types.sort_unstable();
                        let possible_members = Arc::new(member_types.clone());

                        trace!(
                            "Handling a field {}.{}/{} resolving a union type {}",
                            def_name,
                            field_name,
                            graph_id,
                            target_type
                        );

                        for member in member_types.iter().copied() {
                            let tail = self.upsert_node(Node::new_specialized_node(
                                target_type,
                                build_context.resolve_graph_id(state, graph_id)?.clone(),
                                state.is_interface_object_in_subgraph(target_type, graph_id),
                                SubgraphTypeSpecialization::UnionMembers(UnionMembersData {
                                    type_name: def_name,
                                    field_name,
                                    object_type_name: member,
                                    possible_members: Arc::clone(&possible_members),
                                }),
                            ));
                            let abstract_tail = self.upsert_subgraph_node(
                                build_context,
                                state,
                                member,
                                graph_id,
                                state.is_interface_object_in_subgraph(member, graph_id),
                            )?;
                            // because we duplicate tails, we need to add __typename to all of them
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
                                    0,
                                    None,
                                    None,
                                    vec![],
                                    None,
                                ),
                            );

                            trace!(
                                "  [x] Creating field move edge '{}.{}/{}' (type: String)",
                                def_name,
                                field_name,
                                graph_id
                            );
                            let edge = self.push_edge(
                                head,
                                tail,
                                Edge::create_field_move(
                                    field_name.as_str(),
                                    def_name.as_str(),
                                    Self::is_leaf_output_type(state, target_type),
                                    field_definition.field_type.list_depth(),
                                    maybe_join_field,
                                    requirements.clone(),
                                    context_arguments.clone(),
                                    overridden_by.clone(),
                                ),
                            );
                            if let Some(provided) = &provided {
                                provides.insert(edge, provided.clone());
                            }

                            trace!(
                                "  [x] Creating abstract move edge for '{}.{}/{}' (union member: {})",
                                def_name, field_name, graph_id, member
                            );
                            self.push_edge(
                                tail,
                                abstract_tail,
                                Edge::AbstractMove(member.to_string()),
                            );
                            // a conditional `... on Union` keeps the fragment with a selfie, like
                            // any other type, and the path stays on this member's node
                            self.push_edge(tail, tail, Edge::Selfie(target_type.to_string()));
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

                    let tail = self.upsert_subgraph_node(
                        build_context,
                        state,
                        target_type,
                        graph_id,
                        state.is_interface_object_in_subgraph(target_type, graph_id),
                    )?;

                    let edge = self.push_edge(
                        head,
                        tail,
                        Edge::create_field_move(
                            field_name.as_str(),
                            def_name.as_str(),
                            Self::is_leaf_output_type(state, target_type),
                            field_definition.field_type.list_depth(),
                            maybe_join_field,
                            requirements.clone(),
                            context_arguments,
                            overridden_by.clone(),
                        ),
                    );
                    if let Some(provided) = provided {
                        provides.insert(edge, provided);
                    }

                    // If the target type is a root type, we handle it differently and checking if a re-entry is needed.
                    // Our goal is to find all "Query/subgraph" entrypoints
                    if let Some(root_entrypoints) = state
                        .maybe_root_type(target_type)
                        .and_then(|root_kind| get_entrypoints(self, &root_kind).ok())
                        .map(|edge_references| {
                            edge_references
                                .iter()
                                .map(|edge_ref| edge_ref.target())
                                .collect::<Vec<_>>()
                        })
                    {
                        for target_node_index in root_entrypoints {
                            let target_node = self.graph.node_weight(target_node_index).unwrap(); // safe because we know it's already there

                            let Some(graph_id) = target_node.graph_id() else {
                                continue;
                            };

                            if graph_id == graph_name {
                                continue;
                            }

                            trace!(
                                "[x] Creating root re-entry field move edge '{}.{}/{}' (type: {})",
                                def_name,
                                field_name,
                                graph_id,
                                target_node.display_name()
                            );

                            self.upsert_edge(
                                head,
                                target_node_index,
                                Edge::create_reentry_move(
                                    field_name.clone(),
                                    def_name.clone(),
                                    field_definition.field_type.list_depth(),
                                ),
                            );
                        }
                    }
                }
            }
        }

        Ok(provides)
    }
}

impl<'graph> Graph<'graph> {
    /// A `@provides` field makes more fields available below it, but only on that path.
    ///
    /// The path gets its own copies of the nodes it goes through. A copy is keyed by its plain
    /// node and by what's provided on it, and it's built from just those two: the plain node's
    /// edges, pointed at the copies for what's provided further down. So:
    /// - it doesn't matter which `@provides` we get to first,
    /// - two paths that provide the same fields share their copies, different fields never do,
    /// - returning the same type isn't enough to keep what's provided. `User.related: User
    ///   @provides(fields: "name")` comes back to the copy it's on only because its own
    ///   `@provides` makes the same state again. Another `User` field without it goes to the
    ///   plain `User`.
    ///
    /// Runs after all other edges exist, because a copy only gets the edges its plain node has.
    #[instrument(level = "trace", skip(self, state))]
    fn build_provides_edges(
        &mut self,
        state: &'graph SupergraphState,
        provides: HashMap<EdgeIndex, Provided>,
    ) -> Result<(), GraphError> {
        // The map is in no particular order. Sorting keeps the node order, and so the plans,
        // the same between runs.
        let mut providing_edges: Vec<(String, EdgeIndex, String)> = provides
            .keys()
            .map(|&edge| {
                let (head, tail) = self.graph.edge_endpoints(edge).expect("edge exists");
                let Edge::FieldMove(field_move) = &self.graph[edge] else {
                    unreachable!("only field moves have @provides");
                };
                let graph_id = field_move
                    .join_field
                    .as_ref()
                    .and_then(|join_field| join_field.graph_id.clone())
                    .expect("@provides comes with a graph");
                let key = format!(
                    "{} {} {}",
                    self.graph[head], field_move.name, self.graph[tail]
                );
                (key, edge, graph_id)
            })
            .collect();
        providing_edges.sort_unstable();

        let mut ctx = ProvidesContext {
            state,
            provides,
            copies: HashMap::new(),
        };

        // Copies are built from the plain edges, so those stay as they are until every copy
        // exists.
        let mut redirects = Vec::new();
        for (_, edge, graph_id) in providing_edges {
            let target = self.graph.edge_endpoints(edge).expect("edge exists").1;
            let provided = ctx.provides[&edge].clone();
            let copy = self.provided_node(&mut ctx, target, provided, &graph_id)?;
            if copy != target {
                redirects.push((edge, copy));
            }
        }

        // `remove_edge` moves the last edge into the freed index. Going from the highest index
        // down, that's never one we still have to redirect.
        redirects.sort_unstable_by_key(|&(edge, _)| std::cmp::Reverse(edge));
        for (edge, copy) in redirects {
            let head = self.graph.edge_endpoints(edge).expect("edge exists").0;
            let weight = self.graph.remove_edge(edge).expect("edge exists");
            trace!(
                "Pointing provided field '{}.{}' at {}",
                self.graph[head].display_name(),
                weight.display_name(),
                self.graph[copy].display_name()
            );
            self.graph.add_edge(head, copy, weight);
        }

        Ok(())
    }

    /// The node for `original` with `provided` available on it: `original` itself when that
    /// adds nothing, its copy otherwise.
    fn provided_node(
        &mut self,
        ctx: &mut ProvidesContext<'graph>,
        original: NodeIndex,
        provided: Provided,
        graph_id: &str,
    ) -> Result<NodeIndex, GraphError> {
        let provided = self.prune_provided(ctx.state, original, provided, graph_id)?;
        if provided.is_empty() {
            return Ok(original);
        }
        if let Some(&copy) = ctx.copies.get(&(original, provided.clone())) {
            return Ok(copy);
        }

        let copy = self.upsert_node(self.graph[original].provides_copy(provided.to_string()));
        // Saved before the edges are, so a path that comes back to the same state ends up here.
        ctx.copies.insert((original, provided.clone()), copy);

        // `original` is a plain node, so these are plain edges, and `ctx.provides` knows them.
        let edges: Vec<(EdgeIndex, NodeIndex, Edge<'graph>)> = self
            .graph
            .edges(original)
            .map(|edge| (edge.id(), edge.target(), edge.weight().clone()))
            .collect();
        for (edge_index, target, edge) in edges {
            let target = match &edge {
                Edge::FieldMove(field_move) => {
                    let mut below = provided
                        .fields
                        .get(field_move.name)
                        .cloned()
                        .unwrap_or_default();
                    // The field's own `@provides` counts on every path.
                    if let Some(own) = ctx.provides.get(&edge_index) {
                        below.merge(own.clone());
                    }
                    self.provided_node(ctx, target, below, graph_id)?
                }
                Edge::AbstractMove(type_name) => {
                    let below = provided
                        .on_types
                        .get(type_name)
                        .cloned()
                        .unwrap_or_default();
                    self.provided_node(ctx, target, below, graph_id)?
                }
                // `... on Order` on an `Order` keeps the provided fields.
                Edge::Selfie(_) if target == original => copy,
                // Provided fields are gone after an entity call, even to the same subgraph.
                _ => target,
            };
            self.graph.add_edge(copy, target, edge);
        }

        // What's left are `@external` fields and type conditions, only available on this path.
        let type_name = self.graph[original].name_str().to_string();
        let parent_type_name = ctx
            .state
            .definitions
            .get(&type_name)
            .ok_or_else(|| GraphError::DefinitionNotFound(type_name.clone()))?
            .name();
        let subgraph = ctx.state.resolve_graph_id(graph_id)?;
        for (field_name, below) in &provided.fields {
            if self.field_targets(original, field_name).next().is_some() {
                continue;
            }
            let field_definition = field_definition(ctx.state, &type_name, field_name)?;
            let return_type_name = field_definition.field_type.inner_type();
            let plain = self.upsert_node(Node::new_node(
                return_type_name,
                subgraph.clone(),
                ctx.state
                    .is_interface_object_in_subgraph(return_type_name, graph_id),
            ));
            let target = self.provided_node(ctx, plain, below.clone(), graph_id)?;
            self.graph.add_edge(
                copy,
                target,
                Edge::create_field_move(
                    field_definition.name.as_str(),
                    parent_type_name,
                    ctx.state.is_scalar_type(return_type_name),
                    field_definition.field_type.list_depth(),
                    None,
                    None,
                    vec![],
                    None,
                ),
            );
        }
        for (on_type, below) in &provided.on_types {
            if self.member_targets(original, on_type).next().is_some() {
                continue;
            }
            let on_type_name = ctx
                .state
                .definitions
                .get(on_type)
                .ok_or_else(|| GraphError::DefinitionNotFound(on_type.clone()))?
                .name();
            let plain = self.upsert_node(Node::new_node(
                on_type_name,
                subgraph.clone(),
                ctx.state.is_interface_object_in_subgraph(on_type, graph_id),
            ));
            let target = self.provided_node(ctx, plain, below.clone(), graph_id)?;
            self.graph
                .add_edge(copy, target, Edge::AbstractMove(on_type.clone()));
        }

        Ok(copy)
    }

    /// Drops the parts of `provided` that `node` has anyway, and folds `... on T` into the
    /// fields when `node` is a `T`. What's left is the key of the copy.
    fn prune_provided(
        &self,
        state: &SupergraphState,
        node: NodeIndex,
        mut provided: Provided,
        graph_id: &str,
    ) -> Result<Provided, GraphError> {
        // Most edges of a copy have nothing provided below them.
        if provided.is_empty() {
            return Ok(provided);
        }
        let type_name = self.graph[node].name_str().to_string();
        while let Some(same_type) = provided.on_types.remove(&type_name) {
            provided.merge(same_type);
        }

        let mut pruned = Provided::default();
        for (field_name, below) in provided.fields {
            let targets: Vec<NodeIndex> = self.field_targets(node, &field_name).collect();
            if targets.is_empty() {
                // `@external`, the path is the only way to get it.
                let plain = match below.is_empty() {
                    true => None,
                    false => self.plain_node_of_field(state, node, &field_name, graph_id)?,
                };
                let below = match plain {
                    Some(plain) => self.prune_provided(state, plain, below, graph_id)?,
                    None => below,
                };
                pruned.fields.insert(field_name, below);
                continue;
            }
            // The subgraph resolves it anyway, so only what's below can add something.
            let mut kept = Provided::default();
            for target in targets {
                kept.merge(self.prune_provided(state, target, below.clone(), graph_id)?);
            }
            if !kept.is_empty() {
                pruned.fields.insert(field_name, kept);
            }
        }

        for (on_type, below) in provided.on_types {
            let members: Vec<NodeIndex> = self.member_targets(node, &on_type).collect();
            if members.is_empty() {
                // On a union member tail, it's another member's condition.
                if self.graph[node].union_members_data().is_some() {
                    continue;
                }
                // An `@external` abstract field has no member edges here, the path adds one.
                let plain = Node::new_node(
                    &on_type,
                    state.resolve_graph_id(graph_id)?,
                    state.is_interface_object_in_subgraph(&on_type, graph_id),
                );
                let below = match self.node_to_index.get(&plain) {
                    Some(&plain) => self.prune_provided(state, plain, below, graph_id)?,
                    None => below,
                };
                pruned.on_types.insert(on_type, below);
                continue;
            }
            let mut kept = Provided::default();
            for member in members {
                kept.merge(self.prune_provided(state, member, below.clone(), graph_id)?);
            }
            if !kept.is_empty() {
                pruned.on_types.insert(on_type, kept);
            }
        }

        Ok(pruned)
    }

    fn field_targets<'a>(
        &'a self,
        node: NodeIndex,
        field_name: &'a str,
    ) -> impl Iterator<Item = NodeIndex> + 'a {
        self.graph
            .edges(node)
            .filter(move |edge| matches!(edge.weight(), Edge::FieldMove(field_move) if field_move.name == field_name))
            .map(|edge| edge.target())
    }

    fn member_targets<'a>(
        &'a self,
        node: NodeIndex,
        type_name: &'a str,
    ) -> impl Iterator<Item = NodeIndex> + 'a {
        self.graph
            .edges(node)
            .filter(
                move |edge| matches!(edge.weight(), Edge::AbstractMove(name) if name == type_name),
            )
            .map(|edge| edge.target())
    }

    /// The plain node an `@external` field would lead to, if it's in the graph already.
    fn plain_node_of_field(
        &self,
        state: &SupergraphState,
        node: NodeIndex,
        field_name: &str,
        graph_id: &str,
    ) -> Result<Option<NodeIndex>, GraphError> {
        let field_definition = field_definition(state, self.graph[node].name_str(), field_name)?;
        let return_type_name = field_definition.field_type.inner_type();
        let plain = Node::new_node(
            return_type_name,
            state.resolve_graph_id(graph_id)?,
            state.is_interface_object_in_subgraph(return_type_name, graph_id),
        );
        Ok(self.node_to_index.get(&plain).copied())
    }
}

fn field_definition<'a>(
    state: &'a SupergraphState,
    type_name: &str,
    field_name: &str,
) -> Result<&'a SupergraphField, GraphError> {
    state
        .definitions
        .get(type_name)
        .ok_or_else(|| GraphError::DefinitionNotFound(type_name.to_string()))?
        .fields()
        .get(field_name)
        .ok_or_else(|| {
            GraphError::FieldDefinitionNotFound(field_name.to_string(), type_name.to_string())
        })
}

struct ProvidesContext<'a> {
    state: &'a SupergraphState,
    /// The own `@provides` of each plain field edge that has one.
    provides: HashMap<EdgeIndex, Provided>,
    /// Copies, by their plain node and what's provided on them.
    copies: HashMap<(NodeIndex, Provided), NodeIndex>,
}

/// Print me with `println!("{}", graph);` to see the graph in DOT/digraph format.
impl Display for Graph<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", Dot::with_config(&self.graph, &[]))
    }
}

/// The field moves out of each node, sorted by the field name's hash, then its entity moves, all
/// in one list. The walker looks a field up on every path, and a type can have hundreds of fields
/// to go through otherwise. One list instead of a map of lists keeps building the graph cheap.
#[derive(Debug, Default)]
struct EdgeIndexes {
    /// Per node, where its field moves and where its entity moves start in `edges`, and one more
    /// entry for the end.
    starts: Vec<(u32, u32)>,
    /// The field name's hash and the edge. Entity moves have no name, theirs is `NO_NAME`, and
    /// only `fields` looks at the hash.
    edges: Vec<(u64, EdgeIndex)>,
}

const NO_NAME: u64 = 0;

impl EdgeIndexes {
    fn fields(&self, node: NodeIndex) -> &[(u64, EdgeIndex)] {
        match self.starts.get(node.index()) {
            Some(&(fields, entities)) => &self.edges[fields as usize..entities as usize],
            None => &[],
        }
    }

    fn entity_moves(&self, node: NodeIndex) -> impl Iterator<Item = EdgeIndex> + '_ {
        let moves = match (
            self.starts.get(node.index()),
            self.starts.get(node.index() + 1),
        ) {
            (Some(&(_, entities)), Some(&(next, _))) => {
                &self.edges[entities as usize..next as usize]
            }
            _ => &[],
        };
        moves.iter().map(|(_, index)| *index)
    }
}

fn name_hash(name: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = rustc_hash::FxHasher::default();
    name.hash(&mut hasher);
    hasher.finish()
}

/// Object types every subgraph treats the same way, in groups. The same keys and the same
/// subgraphs, the same interfaces and unions in each, the same fields with the same
/// `@join__field` in each: everything the graph is built from but the name. A type that names
/// itself, like `next: Movie`, has nothing in common with another one. The planner plans the same
/// selections on two of them once, see the walker.
fn interchangeable_types(
    state: &SupergraphState,
) -> (HashMap<&str, u32>, HashMap<&str, TypeProfile<'_>>) {
    // a type named in a field set, like `@provides(fields: "... on Book { title }")`, gets
    // something the others don't
    let Some(named_in_field_sets) = types_named_in_field_sets(state) else {
        return Default::default();
    };
    let mut unions: HashMap<&str, Vec<(&str, &str)>> = HashMap::new();
    for (name, definition) in state.definitions.iter() {
        if let SupergraphDefinition::Union(union_type) = definition {
            for member in &union_type.union_members {
                unions
                    .entry(member.member.as_str())
                    .or_default()
                    .push((name.as_str(), member.graph.as_str()));
            }
        }
    }
    for memberships in unions.values_mut() {
        memberships.sort_unstable();
    }
    let mut profiles = HashMap::new();
    // the planner compares these a lot, so each distinct one gets an id
    let mut field_ids = FieldProfileIds::default();
    let mut membership_ids: HashMap<Vec<&str>, u32> = HashMap::new();
    // equal signatures hash the same, so only the ones in a bucket get compared
    let mut buckets: HashMap<u64, Vec<(TypeSignature, Vec<&str>)>> = HashMap::new();
    for (name, definition) in state.definitions.iter() {
        let SupergraphDefinition::Object(object) = definition else {
            continue;
        };
        // an interface object in some subgraph turns into these types with its own moves, and
        // an interface with a key is called as the interface, with a type rewritten per type
        let behind_interface_object = object.join_implements.iter().any(|join| {
            matches!(
                state.definitions.get(&join.interface),
                Some(SupergraphDefinition::Interface(interface))
                    if interface.join_type.iter().any(|join| join.is_interface_object || join.key.is_some())
            )
        });
        if object.root_type.is_some()
            || behind_interface_object
            || named_in_field_sets.contains(name.as_str())
        {
            continue;
        }
        let unions = unions.get(name.as_str()).map_or(&[][..], Vec::as_slice);
        let signature = TypeSignature::of(object, unions);

        if let Some(profile) = type_profile(
            name,
            object,
            unions,
            &signature,
            &mut field_ids,
            &mut membership_ids,
        ) {
            profiles.insert(name.as_str(), profile);
        }

        // a type that names itself can't be swapped for another
        if signature.fields.iter().any(|field| field.names_type(name)) {
            continue;
        }
        let bucket = buckets.entry(signature.bucket()).or_default();
        match bucket.iter_mut().find(|(other, _)| *other == signature) {
            Some((_, names)) => names.push(name.as_str()),
            None => bucket.push((signature, vec![name.as_str()])),
        }
    }
    let mut signatures: Vec<Vec<&str>> = buckets
        .into_values()
        .flatten()
        .map(|(_, names)| names)
        .filter(|names| names.len() > 1)
        .collect();
    signatures.sort();
    let mut groups = HashMap::new();
    for (group, names) in signatures.into_iter().enumerate() {
        for name in names {
            groups.insert(name, group as u32);
        }
    }
    (groups, profiles)
}

/// What `interchangeable_for` compares of `object`. `None` when it has no single key.
fn type_profile<'a>(
    name: &str,
    object: &'a SupergraphObjectType,
    unions: &[(&'a str, &'a str)],
    signature: &TypeSignature<'a>,
    field_ids: &mut FieldProfileIds<'a>,
    membership_ids: &mut HashMap<Vec<&'a str>, u32>,
) -> Option<TypeProfile<'a>> {
    // a subgraph that can't resolve it is never gone into, it's only where objects come from,
    // and they leave with the same key
    let mut keys = object.join_type.iter().map(|join| join.key.as_deref());
    let key = keys.next().flatten();
    if key.is_none() || !keys.all(|other| other == key) {
        return None;
    }
    let subgraphs = object
        .join_type
        .iter()
        .map(|join| {
            let mut memberships: Vec<&str> = object
                .join_implements
                .iter()
                .filter(|implements| implements.graph_id == join.graph_id)
                .map(|implements| implements.interface.as_str())
                .chain(
                    unions
                        .iter()
                        .filter(|(_, graph)| *graph == join.graph_id)
                        .map(|(union, _)| *union),
                )
                .collect();
            memberships.sort_unstable();
            let next = membership_ids.len() as u32;
            (
                join.graph_id.as_str(),
                *membership_ids.entry(memberships).or_insert(next),
            )
        })
        .collect();
    let fields = signature
        .fields
        .iter()
        .filter(|field| !field.names_type(name))
        .map(|field| {
            let mut field = field.clone();
            // in every subgraph that has the type, so in the one the objects are in, and a path
            // there beats any other: which subgraphs those are doesn't matter
            if field.join_fields.is_empty() {
                field.resolvable_in = None;
            }
            (field.name, field_ids.id(field))
        })
        .collect();
    Some(TypeProfile {
        key,
        subgraphs,
        fields,
    })
}

/// See `Graph::required_fields`.
pub(crate) type RequiredFields<'graph> =
    HashMap<&'graph str, HashMap<&'graph str, Option<Vec<String>>>>;

fn required_fields(state: &SupergraphState) -> RequiredFields<'_> {
    let mut required = RequiredFields::new();
    // the same `@requires` on many types gets parsed once
    let mut parsed: HashMap<&str, Option<Vec<String>>> = HashMap::new();
    for (type_name, definition) in state.definitions.iter() {
        for (field_name, field) in definition.fields() {
            let mut all_requires = field
                .join_field
                .iter()
                .filter_map(|join| join.requires.as_deref())
                .peekable();
            if all_requires.peek().is_none() {
                continue;
            }
            let mut fields = Some(Vec::new());
            for requires in all_requires {
                let plain = parsed
                    .entry(requires)
                    .or_insert_with(|| plain_field_names(requires));
                fields = match (fields, plain) {
                    (Some(mut fields), Some(plain)) => {
                        for name in plain.iter() {
                            if !fields.contains(name) {
                                fields.push(name.clone());
                            }
                        }
                        Some(fields)
                    }
                    _ => None,
                };
            }
            required
                .entry(type_name.as_str())
                .or_default()
                .insert(field_name.as_str(), fields);
        }
    }
    required
}

/// The field names in `fields`, when it's only fields without arguments or selections, like
/// `"price currency"`.
fn plain_field_names(fields: &str) -> Option<Vec<String>> {
    FederationRules::parse_field_set(fields)?
        .items
        .into_iter()
        .map(|selection| match selection {
            QuerySelection::Field(field)
                if field.alias.is_none()
                    && field.arguments.is_empty()
                    && field.directives.is_empty()
                    && field.selection_set.items.is_empty() =>
            {
                Some(field.name)
            }
            _ => None,
        })
        .collect()
}

/// The types the `@key`, `@provides`, `@requires` and `@fromContext` field sets name, in a
/// `... on T`. `None` when one doesn't parse, as then there's no telling.
fn types_named_in_field_sets(state: &SupergraphState) -> Option<HashSet<&str>> {
    // most are the same few, like `"id"`, so each gets parsed once
    let mut field_sets = HashSet::new();
    for definition in state.definitions.values() {
        let join_types = match definition {
            SupergraphDefinition::Object(object) => object.join_type.as_slice(),
            SupergraphDefinition::Interface(interface) => interface.join_type.as_slice(),
            _ => &[],
        };
        field_sets.extend(join_types.iter().filter_map(|join| join.key.as_deref()));
        for field in definition.fields().values() {
            for join in &field.join_field {
                field_sets.extend(
                    [&join.provides, &join.requires]
                        .into_iter()
                        .flatten()
                        .map(String::as_str),
                );
                for argument in &join.context_arguments {
                    let selection = argument.selection.trim();
                    field_sets.insert(
                        selection
                            .strip_prefix('{')
                            .and_then(|inner| inner.strip_suffix('}'))
                            .unwrap_or(selection),
                    );
                }
            }
        }
    }
    let mut named = HashSet::new();
    for fields in field_sets {
        collect_type_conditions(
            &FederationRules::parse_field_set(fields)?,
            &mut named,
            state,
        );
    }
    Some(named)
}

fn collect_type_conditions<'s>(
    selection_set: &QuerySelectionSet<'_, String>,
    named: &mut HashSet<&'s str>,
    state: &'s SupergraphState,
) {
    for selection in &selection_set.items {
        match selection {
            QuerySelection::Field(field) => {
                collect_type_conditions(&field.selection_set, named, state)
            }
            QuerySelection::InlineFragment(fragment) => {
                if let Some(QueryTypeCondition::On(type_name)) = &fragment.type_condition {
                    if let Some((name, _)) = state.definitions.get_key_value(type_name) {
                        named.insert(name.as_str());
                    }
                }
                collect_type_conditions(&fragment.selection_set, named, state)
            }
            QuerySelection::FragmentSpread(_) => {}
        }
    }
}

/// Everything about an object type the graph is built from, but its name, see
/// `interchangeable_types`. Lists that come in no particular order are sorted.
#[derive(PartialEq)]
struct TypeSignature<'a> {
    join_type: Vec<&'a JoinTypeDirective>,
    join_implements: Vec<&'a JoinImplementsDirective>,
    unions: Vec<(&'a str, &'a str)>,
    subgraphs: Vec<&'a str>,
    requires_scopes: &'a [RequiresScopesDirective],
    policy: &'a [PolicyDirective],
    authenticated: &'a [AuthenticatedDirective],
    cost: Option<&'a CostDirective>,
    /// By name.
    fields: Vec<FieldProfile<'a>>,
}

impl<'a> TypeSignature<'a> {
    fn of(object: &'a SupergraphObjectType, unions: &[(&'a str, &'a str)]) -> Self {
        let mut join_type: Vec<_> = object.join_type.iter().collect();
        join_type.sort_by(|a, b| {
            (
                &a.graph_id,
                &a.key,
                a.extension,
                a.resolvable,
                a.is_interface_object,
            )
                .cmp(&(
                    &b.graph_id,
                    &b.key,
                    b.extension,
                    b.resolvable,
                    b.is_interface_object,
                ))
        });
        let mut join_implements: Vec<_> = object.join_implements.iter().collect();
        join_implements
            .sort_by(|a, b| (&a.graph_id, &a.interface).cmp(&(&b.graph_id, &b.interface)));
        let mut subgraphs: Vec<_> = object
            .used_in_subgraphs
            .iter()
            .map(String::as_str)
            .collect();
        subgraphs.sort_unstable();
        let mut fields: Vec<_> = object.fields.values().map(FieldProfile::of).collect();
        fields.sort_by(|a, b| a.name.cmp(b.name));
        TypeSignature {
            join_type,
            join_implements,
            unions: unions.to_vec(),
            subgraphs,
            requires_scopes: &object.requires_scopes,
            policy: &object.policy,
            authenticated: &object.authenticated,
            cost: object.cost.as_ref(),
            fields,
        }
    }

    /// A hash of the parts that hash. Equal signatures have equal ones.
    fn bucket(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = rustc_hash::FxHasher::default();
        for join in &self.join_type {
            (&join.graph_id, &join.key).hash(&mut hasher);
        }
        self.subgraphs.hash(&mut hasher);
        for field in &self.fields {
            field.bucket().hash(&mut hasher);
        }
        hasher.finish()
    }
}

/// Everything about a field the graph is built from, see `interchangeable_types`.
#[derive(Clone, PartialEq)]
struct FieldProfile<'a> {
    name: &'a str,
    field_type: &'a TypeNode,
    inaccessible: bool,
    /// By subgraph. Two for one subgraph keep the schema's order, which can only make two
    /// fields look different.
    join_fields: Vec<&'a JoinFieldDirective>,
    /// By name.
    arguments: Vec<(&'a str, &'a TypeNode)>,
    requires_scopes: &'a [RequiresScopesDirective],
    policy: &'a [PolicyDirective],
    authenticated: &'a [AuthenticatedDirective],
    cost: Option<&'a CostDirective>,
    list_size: Option<&'a ListSizeDirective>,
    /// `None` when it doesn't matter, see `interchangeable_types`.
    resolvable_in: Option<&'a [u16]>,
}

impl<'a> FieldProfile<'a> {
    fn of(field: &'a SupergraphField) -> Self {
        let mut join_fields: Vec<_> = field.join_field.iter().collect();
        join_fields.sort_by(|a, b| a.graph_id.cmp(&b.graph_id));
        let mut arguments: Vec<_> = field
            .argument_types
            .iter()
            .map(|(name, type_node)| (name.as_str(), type_node))
            .collect();
        arguments.sort_by(|a, b| a.0.cmp(b.0));
        FieldProfile {
            name: &field.name,
            field_type: &field.field_type,
            inaccessible: field.inaccessible,
            join_fields,
            arguments,
            requires_scopes: &field.requires_scopes,
            policy: &field.policy,
            authenticated: &field.authenticated,
            cost: field.cost.as_ref(),
            list_size: field.list_size.as_ref(),
            resolvable_in: Some(&field.resolvable_in),
        }
    }

    /// A hash of the parts that hash. Equal profiles have equal ones.
    fn bucket(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = rustc_hash::FxHasher::default();
        (self.name, self.field_type, self.resolvable_in).hash(&mut hasher);
        hasher.finish()
    }

    /// Whether it returns `type_name`, or takes or reads it somewhere.
    fn names_type(&self, type_name: &str) -> bool {
        self.field_type.inner_type() == type_name
            || self
                .arguments
                .iter()
                .any(|(_, argument)| argument.inner_type() == type_name)
            || self.join_fields.iter().any(|join| {
                join.type_in_graph
                    .as_ref()
                    .is_some_and(|type_node| type_node.inner_type() == type_name)
                    || join
                        .context_arguments
                        .iter()
                        .any(|argument| argument.type_in_graph.inner_type() == type_name)
            })
    }
}

/// What decides how the planner plans the fields of an object type, field by field, for types
/// that aren't the same everywhere, see `Graph::interchangeable_for`.
#[derive(Debug, Default)]
pub(crate) struct TypeProfile<'graph> {
    /// Its key, when every subgraph that has it keys it with the same fields. A path between
    /// two subgraphs then never needs a third one.
    key: Option<&'graph str>,
    /// The subgraphs that have it, with the interfaces and unions it's in there: the same id
    /// for the same ones.
    subgraphs: HashMap<&'graph str, u32>,
    /// Each field that doesn't name the type, with the id of its `FieldProfile`.
    fields: HashMap<&'graph str, u32>,
}

/// Gives each distinct `FieldProfile` an id, see `TypeProfile::fields`.
#[derive(Default)]
struct FieldProfileIds<'a> {
    /// Equal profiles hash the same, so only the ones in a bucket get compared.
    buckets: HashMap<u64, Vec<(FieldProfile<'a>, u32)>>,
    count: u32,
}

impl<'a> FieldProfileIds<'a> {
    fn id(&mut self, profile: FieldProfile<'a>) -> u32 {
        let bucket = self.buckets.entry(profile.bucket()).or_default();
        if let Some((_, id)) = bucket.iter().find(|(other, _)| *other == profile) {
            return *id;
        }
        let id = self.count;
        self.count += 1;
        bucket.push((profile, id));
        id
    }
}
