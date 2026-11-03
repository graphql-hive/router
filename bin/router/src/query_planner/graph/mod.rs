pub(crate) mod edge;
pub(crate) mod error;
pub(crate) mod node;
mod provided;

pub use self::edge::PlannerOverrideContext;
pub use self::edge::PERCENTAGE_SCALE_FACTOR;

mod tests;

use ahash::{HashMap, HashMapExt, HashSet};
use std::{
    fmt::{Debug, Display},
    sync::Arc,
};

use super::graph::{
    edge::{ContextArgument, Edge, FieldMove},
    node::Node,
    provided::Provided,
};
use crate::query_planner::{
    ast::{
        selection_item::SelectionItem,
        selection_set::{InlineFragmentSelection, SelectionSet},
        type_aware_selection::TypeAwareSelection,
    },
    federation_spec::{directives::JoinFieldDirective, FederationRules},
    graph::node::{SubgraphTypeSpecialization, UnionMembersData},
    state::supergraph_state::{
        OperationKind, SubgraphName as ResolvedSubgraphName, SupergraphDefinition, SupergraphField,
        SupergraphState, SupergraphStateError,
    },
};
use error::GraphError;
use petgraph::{
    dot::Dot,
    graph::{EdgeIndex, Edges, NodeIndex},
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
            let in_subgraphs = registry.entry(def_name.as_str()).or_default();
            for join_member in definition.join_union_members() {
                in_subgraphs
                    .entry(join_member.graph.as_str())
                    .or_default()
                    .insert(join_member.member.as_str());
            }
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
}

impl<'graph> Graph<'graph> {
    #[instrument(level = "trace", skip(supergraph_state))]
    pub fn graph_from_supergraph_state(
        supergraph_state: &'graph SupergraphState,
    ) -> Result<Self, GraphError> {
        GraphBuilder::new(supergraph_state).build()
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

    pub fn root_query_node(&self) -> &Node<'_> {
        &self.graph[self.query_root]
    }

    pub fn root_mutation_node(&self) -> Option<&Node<'_>> {
        self.mutation_root.map(|root| &self.graph[root])
    }

    pub fn root_subscription_node(&self) -> Option<&Node<'_>> {
        self.subscription_root.map(|root| &self.graph[root])
    }

    pub fn edges_to(&self, node_index: NodeIndex) -> Edges<'_, Edge<'graph>, Directed> {
        self.graph.edges_directed(node_index, Direction::Incoming)
    }

    pub fn edges_from(&self, node_index: NodeIndex) -> Edges<'_, Edge<'graph>, Directed> {
        self.graph.edges_directed(node_index, Direction::Outgoing)
    }
}

/// A field of a type in one subgraph.
#[derive(Clone, Copy)]
struct FieldInSubgraph<'a> {
    name: &'a str,
    definition: &'a SupergraphField,
    /// Its @join__field in that subgraph, if it has one.
    join_field: Option<&'a JoinFieldDirective>,
}

/// The fields of a type, sorted into the subgraphs the type is in.
struct SubgraphFields<'a> {
    /// Each subgraph once, by graph id, and whether the type is an @interfaceObject there.
    subgraphs: Vec<(&'a str, bool)>,
    /// The fields of each subgraph, in the order of `subgraphs`.
    fields: Vec<Vec<FieldInSubgraph<'a>>>,
}

impl<'a> SubgraphFields<'a> {
    fn of(definition: &'a SupergraphDefinition) -> Self {
        // A type can repeat @join__type for one subgraph (one per @key), so each subgraph
        // shows up once here. That way every edge below is added once, no dedup needed.
        let mut subgraphs: Vec<(&str, bool)> = definition
            .join_types()
            .iter()
            .map(|join_type| (join_type.graph_id.as_str(), join_type.is_interface_object))
            .collect();
        subgraphs.sort_by_key(|&(graph_id, _)| graph_id);
        subgraphs.dedup_by(|later, kept| {
            let same = later.0 == kept.0;
            if same {
                kept.1 |= later.1;
            }
            same
        });

        // Sort the fields into their subgraphs in one go. Asking every field about every
        // subgraph is subgraphs * fields, and shared types like Query live in hundreds.
        // A field without @join__field lives wherever its type does,
        // otherwise the first @join__field of a subgraph is the one that counts.
        let mut fields: Vec<Vec<FieldInSubgraph>> = vec![Vec::new(); subgraphs.len()];
        for (name, field_definition) in definition.fields() {
            let field = |join_field| FieldInSubgraph {
                name,
                definition: field_definition,
                join_field,
            };
            if field_definition.join_field.is_empty() {
                for fields in &mut fields {
                    fields.push(field(None));
                }
                continue;
            }
            for join_field in &field_definition.join_field {
                let Some(graph_id) = join_field.graph_id.as_deref() else {
                    continue;
                };
                let Ok(position) =
                    subgraphs.binary_search_by_key(&graph_id, |&(graph_id, _)| graph_id)
                else {
                    continue;
                };
                let fields = &mut fields[position];
                if fields
                    .last()
                    .is_some_and(|last| std::ptr::eq(last.definition, field_definition))
                {
                    continue;
                }
                fields.push(field(Some(join_field)));
            }
        }

        Self { subgraphs, fields }
    }
}

/// A type in one subgraph, the head of the field edges being built.
struct TypeInSubgraph<'a> {
    type_name: &'a str,
    graph_id: &'a str,
    /// The subgraph name behind `graph_id`.
    subgraph: &'a str,
    node: NodeIndex,
}

/// Builds a `Graph` out of the supergraph.
///
/// Holds what only the build needs, like the node lookup. It's dropped with the builder,
/// so a built graph doesn't keep a second copy of every node around.
struct GraphBuilder<'graph> {
    state: &'graph SupergraphState,
    graph: InnerGraph<'graph>,
    query_root: NodeIndex,
    mutation_root: Option<NodeIndex>,
    subscription_root: Option<NodeIndex>,
    nodes: HashMap<Node<'graph>, NodeIndex>,
    unions: UnionDefinitions<'graph>,
    /// Fields reading the same context selection in a subgraph share its parsed form.
    context_selections:
        HashMap<(&'graph str, &'graph str, &'graph str), Arc<[TypeAwareSelection<'graph>]>>,
    /// The own `@provides` of each field edge that has one, for `build_provides_edges`.
    provides: HashMap<EdgeIndex, Provided>,
}

impl<'graph> GraphBuilder<'graph> {
    fn new(state: &'graph SupergraphState) -> Self {
        Self {
            state,
            graph: InnerGraph::new(),
            query_root: NodeIndex::default(),
            mutation_root: None,
            subscription_root: None,
            nodes: HashMap::new(),
            unions: UnionDefinitions::new(state),
            context_selections: HashMap::new(),
            provides: HashMap::new(),
        }
    }

    #[instrument(level = "trace", skip(self))]
    fn build(mut self) -> Result<Graph<'graph>, GraphError> {
        trace!(
            "Building graph for supergraph with {} definitions",
            self.state.definitions.len()
        );

        self.build_root_nodes();
        self.link_root_edges()?;
        self.build_field_edges()?;
        // Only field edges need these, no point holding them for the rest of the build.
        self.unions = UnionDefinitions::default();
        self.context_selections = HashMap::new();
        self.build_interface_implementation_edges()?;
        self.build_entity_reference_edges()?;
        self.build_provides_edges()?;

        for node in self.graph.node_weights_mut() {
            if let Node::SubgraphType(subgraph_type) = node {
                let name = subgraph_type.subgraph;
                subgraph_type.subgraph_index = self
                    .state
                    .subgraph_index(name)
                    .ok_or_else(|| SupergraphStateError::SubgraphNotFound(name.to_string()))?;
            }
        }

        Ok(Graph {
            graph: self.graph,
            query_root: self.query_root,
            mutation_root: self.mutation_root,
            subscription_root: self.subscription_root,
        })
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

    fn is_leaf_output_type(&self, type_name: &str) -> bool {
        self.state.is_scalar_type(type_name)
            || self
                .state
                .definitions
                .get(type_name)
                .is_some_and(|definition| matches!(definition, SupergraphDefinition::Enum(_)))
    }

    /// The subgraph name behind a `join__Graph` value.
    fn subgraph_name(&self, graph_id: &str) -> Result<&'graph str, GraphError> {
        Ok(self
            .state
            .known_subgraphs
            .get(graph_id)
            .ok_or_else(|| SupergraphStateError::SubgraphNotFound(graph_id.to_string()))?)
    }

    fn field_definition(
        &self,
        type_name: &str,
        field_name: &str,
    ) -> Result<&'graph SupergraphField, GraphError> {
        self.state
            .definitions
            .get(type_name)
            .ok_or_else(|| GraphError::DefinitionNotFound(type_name.to_string()))?
            .fields()
            .get(field_name)
            .ok_or_else(|| {
                GraphError::FieldDefinitionNotFound(field_name.to_string(), type_name.to_string())
            })
    }

    /// The index of `node`, added if it's not in the graph yet.
    fn node(&mut self, node: Node<'graph>) -> NodeIndex {
        if let Some(&index) = self.nodes.get(&node) {
            return index;
        }

        let index = self.graph.add_node(node);
        self.nodes.insert(self.graph[index].clone(), index);

        index
    }

    fn subgraph_node(
        &mut self,
        type_name: &'graph str,
        graph_id: &str,
        is_interface_object: bool,
    ) -> Result<NodeIndex, GraphError> {
        let subgraph = self.subgraph_name(graph_id)?;
        Ok(self.node(Node::new_node(type_name, subgraph, is_interface_object)))
    }

    /// Like `subgraph_node`, with the supergraph saying if it's an @interfaceObject.
    fn type_node(
        &mut self,
        type_name: &'graph str,
        graph_id: &str,
    ) -> Result<NodeIndex, GraphError> {
        let is_interface_object = self
            .state
            .is_interface_object_in_subgraph(type_name, graph_id);
        self.subgraph_node(type_name, graph_id, is_interface_object)
    }

    /// Adds the edges in order, but only the first of any that repeat between the same nodes.
    /// Repeats come from a type repeating @join__type in a subgraph, one per @key.
    /// Sorting finds them. Checking each new edge against all edges of its head
    /// is subgraphs^2 * edges per type, and a node has an edge per field.
    ///
    /// Only for edges that are equal when their names are, like entity and abstract moves.
    fn push_unique_edges(&mut self, edges: Vec<(NodeIndex, NodeIndex, Edge<'graph>)>) {
        // Equal edges have to end up next to each other, so the kind goes before the name.
        let sort_key = |index: usize| {
            let (head, tail, edge) = &edges[index];
            let is_entity_move = matches!(edge, Edge::EntityMove(_));
            (*head, *tail, is_entity_move, edge.display_name(), index)
        };
        let mut first_ones: Vec<usize> = (0..edges.len()).collect();
        first_ones.sort_unstable_by(|&a, &b| sort_key(a).cmp(&sort_key(b)));
        first_ones.dedup_by(|later, earlier| {
            let (head, tail, edge) = &edges[*later];
            let (first_head, first_tail, first_edge) = &edges[*earlier];
            head == first_head && tail == first_tail && edge == first_edge
        });
        first_ones.sort_unstable();

        let mut first_ones = first_ones.into_iter().peekable();
        for (index, (head, tail, edge)) in edges.into_iter().enumerate() {
            if first_ones.next_if_eq(&index).is_some() {
                self.graph.add_edge(head, tail, edge);
            }
        }
    }

    #[instrument(level = "trace", skip(self))]
    fn build_root_nodes(&mut self) {
        let state = self.state;
        self.query_root = self.node(Node::QueryRoot(state.query_type.as_str()));
        trace!("added root type for queries: {}", state.query_type);
        self.mutation_root = state.mutation_type.as_ref().map(|mutation_type| {
            trace!("added root type for mutations: {}", mutation_type);
            self.node(Node::MutationRoot(mutation_type.as_str()))
        });
        self.subscription_root = state.subscription_type.as_ref().map(|subscription_type| {
            trace!("added root type for subscriptions: {}", subscription_type);
            self.node(Node::SubscriptionRoot(subscription_type.as_str()))
        });
    }

    fn root(&self, kind: &OperationKind) -> Option<NodeIndex> {
        match kind {
            OperationKind::Query => Some(self.query_root),
            OperationKind::Mutation => self.mutation_root,
            OperationKind::Subscription => self.subscription_root,
        }
    }

    #[instrument(level = "trace", skip(self))]
    fn link_root_edges(&mut self) -> Result<(), GraphError> {
        let state = self.state;
        for (def_name, definition) in state
            .definitions
            .iter()
            .filter(|(_, definition)| Self::needs_output_traversal(definition))
        {
            let Some(root_type) = definition.try_into_root_type() else {
                continue;
            };
            // `subgraphs()` is sorted but may contain duplicate graph IDs if the SDL has
            // duplicate `@join__type`s. A root entrypoint is unique per root and graph.
            let mut graph_ids = definition.subgraphs();
            graph_ids.dedup();

            // Root entrypoints only need to know whether each graph can resolve anything.
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
                if graphs_without_fields.contains(graph_id) {
                    continue;
                }
                let head = self
                    .root(root_type)
                    .ok_or(GraphError::MissingRootType(root_type.clone()))?;
                let subgraph = self.subgraph_name(graph_id)?;
                let tail = self.node(Node::new_root_node(def_name, subgraph));

                self.graph.add_edge(
                    head,
                    tail,
                    Edge::SubgraphEntrypoint {
                        name: ResolvedSubgraphName(subgraph.to_string()),
                        operation_kind: root_type.clone(),
                    },
                );
            }
        }

        Ok(())
    }

    #[instrument(level = "trace", skip(self))]
    fn build_field_edges(&mut self) -> Result<(), GraphError> {
        let state = self.state;
        for (def_name, definition) in state
            .definitions
            .iter()
            .filter(|(_, definition)| Self::needs_output_traversal(definition))
        {
            let SubgraphFields { subgraphs, fields } = SubgraphFields::of(definition);
            for ((graph_id, is_interface_object), fields) in subgraphs.into_iter().zip(fields) {
                // __typename is not resolvable for @interfaceObject
                if !is_interface_object {
                    trace!(
                        "[x] Creating owned field move edge '{}.__typename/{}' (type: String)",
                        def_name,
                        graph_id
                    );
                    let head = self.subgraph_node(def_name, graph_id, false)?;
                    let tail = self.subgraph_node("String", graph_id, false)?;
                    self.graph
                        .add_edge(head, tail, FieldMove::typename(def_name).into());
                }

                trace!(
                    "[x] Creating self-referencing edge for '{}/{}'",
                    def_name,
                    graph_id
                );
                let head = self.type_node(def_name, graph_id)?;
                self.graph.add_edge(head, head, Edge::Selfie(def_name));

                let parent = TypeInSubgraph {
                    type_name: def_name,
                    graph_id,
                    subgraph: self.subgraph_name(graph_id)?,
                    node: head,
                };
                for field in fields {
                    self.add_field_edges(&parent, field)?;
                }
            }
        }

        Ok(())
    }

    fn add_field_edges(
        &mut self,
        parent: &TypeInSubgraph<'graph>,
        field: FieldInSubgraph<'graph>,
    ) -> Result<(), GraphError> {
        let is_external = field
            .join_field
            .is_some_and(|join_field| join_field.external && join_field.requires.is_none());
        if is_external {
            trace!(
                "[ ] Field '{}.{}/{}' is external, skipping edge creation",
                parent.type_name,
                field.name,
                parent.graph_id
            );
            return Ok(());
        }

        let target_type = field.definition.field_type.inner_type();
        let field_move = self.field_move(parent, field, target_type);

        // Parsed once, a union field has an edge per member.
        let provided = field
            .join_field
            .and_then(|join_field| {
                let graph_id = join_field.graph_id.as_ref()?;
                FederationRules::parse_provides(self.state, join_field, graph_id, target_type)
            })
            .map(|selection_set| Provided::from_selection_set(&selection_set));

        if self.unions.contains(target_type) {
            return self.add_union_field_edges(parent, field, field_move, provided);
        }

        trace!(
            "[x] Creating field move edge '{}.{}/{}' (type: {})",
            parent.type_name,
            field.name,
            parent.graph_id,
            target_type
        );
        let tail = self.type_node(target_type, parent.graph_id)?;
        let edge = self.graph.add_edge(parent.node, tail, field_move.into());
        if let Some(provided) = provided {
            self.provides.insert(edge, provided);
        }

        self.add_reentry_edges(parent, field, target_type);
        Ok(())
    }

    /// The field move of `field` from `parent`, with its requirements, context arguments
    /// and overrides.
    fn field_move(
        &mut self,
        parent: &TypeInSubgraph<'graph>,
        field: FieldInSubgraph<'graph>,
        target_type: &str,
    ) -> FieldMove<'graph> {
        let state = self.state;

        // A field is considered "overridden" if its resolution is handled by a different subgraph.
        // This prevents the current subgraph from creating a resolvable edge for a field it no longer owns.
        let overridden_by = field.definition.join_field.iter().find_map(|jf| {
            if jf.override_value.as_deref() != Some(parent.subgraph) {
                return None;
            }
            let overriding_subgraph_name = state
                .known_subgraphs
                .get(
                    jf.graph_id
                        .as_ref()
                        .expect("@override must be on a @join__field with a graph argument"),
                )
                .expect("@override graph should be a known subgraph");
            Some((
                overriding_subgraph_name.as_str(),
                jf.override_label.as_ref(),
            ))
        });

        let requirements = field.join_field.and_then(|join_field| {
            join_field.requires.as_ref().map(|requires_str| {
                Arc::new(FederationRules::parse_key(
                    state,
                    join_field.graph_id.as_ref().expect(
                        "join__field(graph:) should exist when join__field(requires:) exists",
                    ),
                    parent.type_name,
                    requires_str,
                ))
            })
        });

        let context_arguments = field
            .join_field
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
                        selections: self
                            .context_selections
                            .entry((graph_id, &argument.context, &argument.selection))
                            .or_insert_with(|| {
                                context_selections(
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

        FieldMove {
            name: field.name,
            type_name: parent.type_name,
            is_leaf: self.is_leaf_output_type(target_type),
            is_list: field.definition.field_type.is_list(),
            join_field: field.join_field,
            requirements,
            context_arguments,
            overridden_by,
        }
    }

    /// A field returning a union gets an edge per member it can return in this subgraph.
    ///
    /// ```graphql
    /// type Viewer @join__type(graph: A) @join__type(graph: B) {
    ///   media: ViewerMedia
    ///   aMedia: ViewerMedia @join__field(graph: A)
    ///   bMedia: ViewerMedia @join__field(graph: B)
    ///   book: ViewerMedia @join__field(graph: A, type: "Book") @join__field(graph: B, type: "ViewerMedia")
    ///   song: ViewerMedia @join__field(graph: A)
    /// }
    ///
    /// union ViewerMedia
    ///   @join__type(graph: A)
    ///   @join__type(graph: B)
    ///   @join__unionMember(graph: A, member: "Book")
    ///   @join__unionMember(graph: B, member: "Book")
    ///   @join__unionMember(graph: A, member: "Song")
    ///   @join__unionMember(graph: B, member: "Movie") =
    ///   | Book
    ///   | Song
    ///   | Movie
    /// ```
    ///
    /// - Viewer.media  (A,B)   = Book            (product of the intersection of A and B)
    /// - Viewer.aMedia (A)     = Book | Song     (no intersection - it lives in a single subgraph)
    /// - Viewer.bMedia (A)     = Book | Movie    (no intersection - it lives in a single subgraph)
    /// - Viewer.book   (A,B)   = Book            (product of the intersection of A and B)
    /// - Viewer.song   (A)     = Book | Sing     (no intersection - it lives in a single subgraph)
    ///
    /// We need to point it to a subset of object types.
    /// We do it by creating a new Node for each edge's tail,
    /// and from the tail we create abstract-move edges to the object types.
    fn add_union_field_edges(
        &mut self,
        parent: &TypeInSubgraph<'graph>,
        field: FieldInSubgraph<'graph>,
        field_move: FieldMove<'graph>,
        provided: Option<Provided>,
    ) -> Result<(), GraphError> {
        let graph_id = parent.graph_id;
        let target_type = field.definition.field_type.inner_type();
        // Build union-member edges for the current subgraph only. Doing a global
        // intersection here strips valid members from pinned paths such as
        // Query.getResponse -> Response/A -> actions, where A knows the full union.
        let mut member_types = self
            .unions
            .members_for_field_in_graph(field.definition, target_type, graph_id)
            .into_iter()
            .collect::<Vec<_>>();
        member_types.sort_unstable();
        let possible_members = Arc::new(member_types.clone());

        trace!(
            "Handling a field {}.{}/{} resolving a union type {}",
            parent.type_name,
            field.name,
            graph_id,
            target_type
        );

        for member in member_types {
            let tail = self.node(Node::new_specialized_node(
                target_type,
                parent.subgraph,
                self.state
                    .is_interface_object_in_subgraph(target_type, graph_id),
                SubgraphTypeSpecialization::UnionMembers(UnionMembersData {
                    type_name: parent.type_name,
                    field_name: field.name,
                    object_type_name: member,
                    possible_members: Arc::clone(&possible_members),
                }),
            ));
            let abstract_tail = self.type_node(member, graph_id)?;
            // because we duplicate tails, we need to add __typename to all of them
            let typename_tail = self.subgraph_node("String", graph_id, false)?;

            trace!(
                "  [x] Creating field move edge '{}.__typename/{}' (type: String)",
                parent.type_name,
                graph_id
            );
            self.graph
                .add_edge(tail, typename_tail, FieldMove::typename(target_type).into());

            trace!(
                "  [x] Creating field move edge '{}.{}/{}' (type: String)",
                parent.type_name,
                field.name,
                graph_id
            );
            let edge = self
                .graph
                .add_edge(parent.node, tail, field_move.clone().into());
            if let Some(provided) = &provided {
                self.provides.insert(edge, provided.clone());
            }

            trace!(
                "  [x] Creating abstract move edge for '{}.{}/{}' (union member: {})",
                parent.type_name,
                field.name,
                graph_id,
                member
            );
            self.graph
                .add_edge(tail, abstract_tail, Edge::AbstractMove(member));
        }

        Ok(())
    }

    /// A field returning a root type can go back in through the root, to any other subgraph.
    fn add_reentry_edges(
        &mut self,
        parent: &TypeInSubgraph<'graph>,
        field: FieldInSubgraph<'graph>,
        target_type: &str,
    ) {
        let Some(root) = self
            .state
            .maybe_root_type(target_type)
            .and_then(|kind| self.root(&kind))
        else {
            return;
        };

        // By edge index, so the edges go in the same order on every build.
        let mut entrypoints: Vec<(EdgeIndex, NodeIndex)> = self
            .graph
            .edges(root)
            .map(|edge| (edge.id(), edge.target()))
            .collect();
        entrypoints.sort_unstable();

        for (_, entrypoint) in entrypoints {
            let target_node = &self.graph[entrypoint];
            let Some(graph_id) = target_node.graph_id() else {
                continue;
            };
            if graph_id == parent.subgraph {
                continue;
            }

            trace!(
                "[x] Creating root re-entry field move edge '{}.{}/{}' (type: {})",
                parent.type_name,
                field.name,
                graph_id,
                target_node.display_name()
            );

            self.graph.add_edge(
                parent.node,
                entrypoint,
                Edge::create_reentry_move(
                    field.name,
                    parent.type_name,
                    field.definition.field_type.is_list(),
                ),
            );
        }
    }

    #[instrument(level = "trace", skip(self))]
    fn build_interface_implementation_edges(&mut self) -> Result<(), GraphError> {
        let state = self.state;
        for (def_name, definition) in state
            .definitions
            .iter()
            .filter(|(_, d)| matches!(d, SupergraphDefinition::Object(_)))
        {
            let mut edges = Vec::new();
            for join_implements in definition.join_implements() {
                // The definitions are object types, so neither end can be an @interfaceObject
                // (it'd have to be an interface).
                let tail = self.subgraph_node(def_name, &join_implements.graph_id, false)?;
                let head = self.subgraph_node(
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

                edges.push((head, tail, Edge::AbstractMove(definition.name())));
            }
            self.push_unique_edges(edges);
        }

        Ok(())
    }

    #[instrument(level = "trace", skip(self))]
    fn build_entity_reference_edges(&mut self) -> Result<(), GraphError> {
        let state = self.state;
        for (def_name, definition) in state
            .definitions
            .iter()
            .filter(|(_, definition)| Self::can_have_entity_moves(definition))
        {
            let is_interface = definition.is_interface_type();
            let join_types = definition.join_types();
            let mut edges = Vec::new();

            let mut nodes = Vec::with_capacity(join_types.len());
            for join_type in join_types {
                nodes.push(self.subgraph_node(
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
                        let key_selection = Arc::new(FederationRules::parse_key(
                            state,
                            &join_type.graph_id,
                            def_name,
                            key,
                        ));
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

                    edges.push((
                        head,
                        tail,
                        Edge::create_entity_move(key, key_selection.clone(), is_interface),
                    ));
                }

                // Connects object types implementing @interfaceObject by @key
                if !join_type1.is_interface_object {
                    continue;
                }

                // Ignore if the @key is not resolable or there is no @key
                let Some((key, key_selection)) = keys[index1].clone() else {
                    continue;
                };

                let interface_object_name = def_name;
                let tail = head;

                let typename_selection = Arc::new(FederationRules::parse_key(
                    state,
                    &join_type1.graph_id,
                    interface_object_name,
                    "__typename",
                ));

                for (object_type_name, object_type_definition) in state.definitions.iter() {
                    let SupergraphDefinition::Object(object_type) = object_type_definition else {
                        continue;
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
                    edges.push((
                        tail,
                        tail,
                        Edge::create_interface_object_type_move(
                            object_type_name,
                            typename_selection.clone(),
                        ),
                    ));

                    // Connect them via @key of the @interfaceObject.
                    for join_type2 in object_type_definition.join_types() {
                        if join_type1.graph_id == join_type2.graph_id {
                            // it shouldn't really happen as the @interfaceObject is an object type,
                            // so no object types within the same subgraph can implement it,
                            // as it's not an interface.
                            continue;
                        }

                        let head = self.subgraph_node(
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

                        edges.push((
                            head,
                            tail,
                            Edge::create_entity_move(key, key_selection.clone(), is_interface),
                        ));
                    }
                }
            }

            self.push_unique_edges(edges);
        }

        Ok(())
    }
}

/// The selection a context argument reads, parsed for each type declaring that context.
fn context_selections<'graph>(
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

/// Copies of nodes made for `@provides` paths, by their plain node and what's provided on them.
type ProvidedCopies = HashMap<(NodeIndex, Provided), NodeIndex>;

impl<'graph> GraphBuilder<'graph> {
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
    #[instrument(level = "trace", skip(self))]
    fn build_provides_edges(&mut self) -> Result<(), GraphError> {
        // The map is in no particular order. Sorting keeps the node order, and so the plans,
        // the same between runs.
        let mut providing_edges: Vec<(String, EdgeIndex, String)> = self
            .provides
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

        // Copies are built from the plain edges, so those stay as they are until every copy
        // exists.
        let mut copies = ProvidedCopies::new();
        let mut redirects = Vec::new();
        for (_, edge, graph_id) in providing_edges {
            let target = self.graph.edge_endpoints(edge).expect("edge exists").1;
            let provided = self.provides[&edge].clone();
            let copy = self.provided_node(&mut copies, target, provided, &graph_id)?;
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
        copies: &mut ProvidedCopies,
        original: NodeIndex,
        provided: Provided,
        graph_id: &str,
    ) -> Result<NodeIndex, GraphError> {
        let provided = self.prune_provided(original, provided, graph_id)?;
        if provided.is_empty() {
            return Ok(original);
        }
        if let Some(&copy) = copies.get(&(original, provided.clone())) {
            return Ok(copy);
        }

        let copy = self.node(self.graph[original].provides_copy(provided.to_string()));
        // Saved before the edges are, so a path that comes back to the same state ends up here.
        copies.insert((original, provided.clone()), copy);

        // `original` is a plain node, so these are plain edges, and `self.provides` knows them.
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
                    if let Some(own) = self.provides.get(&edge_index) {
                        below.merge(own.clone());
                    }
                    self.provided_node(copies, target, below, graph_id)?
                }
                Edge::AbstractMove(type_name) => {
                    let below = provided
                        .on_types
                        .get(*type_name)
                        .cloned()
                        .unwrap_or_default();
                    self.provided_node(copies, target, below, graph_id)?
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
        let parent_type_name = self
            .state
            .definitions
            .get(&type_name)
            .ok_or_else(|| GraphError::DefinitionNotFound(type_name.clone()))?
            .name();
        for (field_name, below) in &provided.fields {
            if self.field_targets(original, field_name).next().is_some() {
                continue;
            }
            let field_definition = self.field_definition(&type_name, field_name)?;
            let return_type_name = field_definition.field_type.inner_type();
            let plain = self.type_node(return_type_name, graph_id)?;
            let target = self.provided_node(copies, plain, below.clone(), graph_id)?;
            self.graph.add_edge(
                copy,
                target,
                FieldMove::plain(
                    field_definition.name.as_str(),
                    parent_type_name,
                    self.state.is_scalar_type(return_type_name),
                    field_definition.field_type.is_list(),
                )
                .into(),
            );
        }
        for (on_type, below) in &provided.on_types {
            if self.member_targets(original, on_type).next().is_some() {
                continue;
            }
            let on_type_name = self
                .state
                .definitions
                .get(on_type)
                .ok_or_else(|| GraphError::DefinitionNotFound(on_type.clone()))?
                .name();
            let plain = self.type_node(on_type_name, graph_id)?;
            let target = self.provided_node(copies, plain, below.clone(), graph_id)?;
            self.graph
                .add_edge(copy, target, Edge::AbstractMove(on_type_name));
        }

        Ok(copy)
    }

    /// Drops the parts of `provided` that `node` has anyway, and folds `... on T` into the
    /// fields when `node` is a `T`. What's left is the key of the copy.
    fn prune_provided(
        &self,
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
                    false => self.plain_node_of_field(node, &field_name, graph_id)?,
                };
                let below = match plain {
                    Some(plain) => self.prune_provided(plain, below, graph_id)?,
                    None => below,
                };
                pruned.fields.insert(field_name, below);
                continue;
            }
            // The subgraph resolves it anyway, so only what's below can add something.
            let mut kept = Provided::default();
            for target in targets {
                kept.merge(self.prune_provided(target, below.clone(), graph_id)?);
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
                    self.subgraph_name(graph_id)?,
                    self.state
                        .is_interface_object_in_subgraph(&on_type, graph_id),
                );
                let below = match self.nodes.get(&plain) {
                    Some(&plain) => self.prune_provided(plain, below, graph_id)?,
                    None => below,
                };
                pruned.on_types.insert(on_type, below);
                continue;
            }
            let mut kept = Provided::default();
            for member in members {
                kept.merge(self.prune_provided(member, below.clone(), graph_id)?);
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
                move |edge| matches!(edge.weight(), Edge::AbstractMove(name) if *name == type_name),
            )
            .map(|edge| edge.target())
    }

    /// The plain node an `@external` field would lead to, if it's in the graph already.
    fn plain_node_of_field(
        &self,
        node: NodeIndex,
        field_name: &str,
        graph_id: &str,
    ) -> Result<Option<NodeIndex>, GraphError> {
        let field_definition = self.field_definition(self.graph[node].name_str(), field_name)?;
        let return_type_name = field_definition.field_type.inner_type();
        let plain = Node::new_node(
            return_type_name,
            self.subgraph_name(graph_id)?,
            self.state
                .is_interface_object_in_subgraph(return_type_name, graph_id),
        );
        Ok(self.nodes.get(&plain).copied())
    }
}

/// Print me with `println!("{}", graph);` to see the graph in DOT/digraph format.
impl Display for Graph<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", Dot::with_config(&self.graph, &[]))
    }
}
