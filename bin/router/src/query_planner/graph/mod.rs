pub(crate) mod edge;
pub(crate) mod error;
pub(crate) mod node;
mod provided;

pub use self::edge::PlannerOverrideContext;
pub use self::edge::PERCENTAGE_SCALE_FACTOR;

mod tests;

use std::{
    collections::{HashMap, HashSet},
    fmt::{Debug, Display},
};

use super::graph::{edge::Edge, node::Node, provided::Provided};
use crate::query_planner::planner::walker::utils::get_entrypoints;
use crate::query_planner::{
    ast::type_aware_selection::TypeAwareSelection,
    federation_spec::FederationRules,
    graph::node::{SubgraphTypeSpecialization, UnionMembersData},
    state::supergraph_state::{
        OperationKind, SupergraphDefinition, SupergraphField, SupergraphState,
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

type InnerGraph = Petgraph<Node, Edge, Directed>;

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
pub struct Graph {
    pub graph: InnerGraph,
    pub query_root: NodeIndex,
    pub mutation_root: Option<NodeIndex>,
    pub subscription_root: Option<NodeIndex>,
    pub node_display_name_to_index: HashMap<String, NodeIndex>,
}

impl Graph {
    #[instrument(level = "trace", skip(supergraph_state))]
    pub fn graph_from_supergraph_state(
        supergraph_state: &SupergraphState,
    ) -> Result<Self, GraphError> {
        let mut instance = Graph {
            node_display_name_to_index: HashMap::new(),
            graph: InnerGraph::new(),
            ..Default::default()
        };

        instance.build_graph(supergraph_state)?;

        Ok(instance)
    }

    pub fn node(&self, node_index: NodeIndex) -> Result<&Node, GraphError> {
        self.graph
            .node_weight(node_index)
            .ok_or(GraphError::NodeNotFound(node_index))
    }

    pub fn edge(&self, edge_index: EdgeIndex) -> Result<&Edge, GraphError> {
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
    fn build_graph(&mut self, state: &SupergraphState) -> Result<(), GraphError> {
        trace!(
            "Building graph for supergraph with {} definitions",
            state.definitions.len()
        );

        self.build_root_nodes(state)?;
        self.link_root_edges(state)?;
        let provides = self.build_field_edges(state)?;
        self.build_interface_implementation_edges(state)?;
        self.build_entity_reference_edges(state)?;
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
    fn build_root_nodes(&mut self, state: &SupergraphState) -> Result<(), GraphError> {
        self.query_root = self.upsert_node(Node::QueryRoot(state.query_type.clone()));
        trace!("added root type for queries: {}", state.query_type);
        self.mutation_root = state.mutation_type.as_ref().map(|mutation_type| {
            trace!("added root type for mutations: {}", mutation_type);
            self.upsert_node(Node::MutationRoot(mutation_type.clone()))
        });
        self.subscription_root = state.subscription_type.as_ref().map(|subscription_type| {
            trace!("added root type for subscriptions: {}", subscription_type);
            self.upsert_node(Node::SubscriptionRoot(subscription_type.clone()))
        });

        Ok(())
    }

    pub fn upsert_node(&mut self, node: Node) -> NodeIndex {
        let display_identifier = node.display_name();

        if let Some(index) = self.node_display_name_to_index.get(&display_identifier) {
            return *index;
        }

        let index = self.graph.add_node(node);
        self.node_display_name_to_index
            .insert(display_identifier, index);

        index
    }

    pub fn upsert_edge(&mut self, head: NodeIndex, tail: NodeIndex, edge: Edge) -> EdgeIndex {
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

    // __typename is a meta-field: it carries no declared field definition, so unlike
    // other fields it always resolves to a plain "String" move, regardless of parent type.
    fn upsert_typename_edge(&mut self, head: NodeIndex, tail: NodeIndex, parent_type_name: &str) {
        self.upsert_edge(
            head,
            tail,
            Edge::create_field_move(
                "__typename".to_string(),
                parent_type_name.to_string(),
                true,
                false,
                None,
                None,
                None,
            ),
        );
    }

    #[instrument(level = "trace", skip(self, state))]
    fn build_entity_reference_edges(&mut self, state: &SupergraphState) -> Result<(), GraphError> {
        for (def_name, definition) in state.definitions.iter() {
            let is_interface = definition.is_interface_type();
            for join_type1 in definition.join_types() {
                // Connects object and interface entities of the same name by @key
                for join_type2 in definition.join_types() {
                    let head = self.upsert_node(Node::new_node(
                        def_name,
                        state.resolve_graph_id(&join_type1.graph_id)?,
                        join_type1.is_interface_object,
                    ));

                    if join_type1.graph_id != join_type2.graph_id {
                        if let (true, Some(key)) = (&join_type2.resolvable, &join_type2.key) {
                            let tail = self.upsert_node(Node::new_node(
                                def_name,
                                state.resolve_graph_id(&join_type2.graph_id)?,
                                join_type2.is_interface_object,
                            ));
                            let key_selection = FederationRules::parse_key(
                                state,
                                &join_type2.graph_id,
                                def_name,
                                key,
                            );

                            trace!(
                                "Creating entity move edge from '{}/{}' to '{}/{}' via key '{}'",
                                def_name,
                                join_type1.graph_id,
                                def_name,
                                join_type2.graph_id,
                                key
                            );

                            self.upsert_edge(
                                head,
                                tail,
                                Edge::create_entity_move(key, key_selection, is_interface),
                            );
                        }
                    } else if let (true, Some(key)) = (&join_type1.resolvable, &join_type1.key) {
                        let key_selection =
                            FederationRules::parse_key(state, &join_type1.graph_id, def_name, key);

                        trace!(
                            "Creating self-referencing entity move edge in '{}/{}' via key '{}'",
                            def_name,
                            join_type1.graph_id,
                            key
                        );

                        self.upsert_edge(
                            head,
                            head,
                            Edge::create_entity_move(key, key_selection, is_interface),
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
                let tail = self.upsert_node(Node::new_node(
                    interface_object_name,
                    state.resolve_graph_id(&join_type1.graph_id)?,
                    join_type1.is_interface_object,
                ));

                let typename_selection = FederationRules::parse_key(
                    state,
                    &join_type1.graph_id,
                    interface_object_name,
                    &"__typename".to_string(),
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

                    let key_selection = FederationRules::parse_key(
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

                        let head = self.upsert_node(Node::new_node(
                            object_type_name,
                            state.resolve_graph_id(&join_type2.graph_id)?,
                            join_type2.is_interface_object,
                        ));

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

    #[instrument(level = "trace", skip(self, state))]
    fn build_interface_implementation_edges(
        &mut self,
        state: &SupergraphState,
    ) -> Result<(), GraphError> {
        for (def_name, definition) in state
            .definitions
            .iter()
            .filter(|(_, d)| matches!(d, SupergraphDefinition::Object(_)))
        {
            for join_implements in definition.join_implements() {
                let tail = self.upsert_node(Node::new_node(
                    def_name,
                    state.resolve_graph_id(&join_implements.graph_id)?,
                    // The definition are object types,
                    // so it can't be @interfaceObject (it'd has be Interface).
                    false,
                ));
                let head = self.upsert_node(Node::new_node(
                    &join_implements.interface,
                    state.resolve_graph_id(&join_implements.graph_id)?,
                    // The definition are object types,
                    // so it can't be @interfaceObject (it'd has be Interface).
                    false,
                ));

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

    pub fn root_query_node(&self) -> &Node {
        &self.graph[self.query_root]
    }

    pub fn root_mutation_node(&self) -> Option<&Node> {
        if let Some(mutation_root) = self.mutation_root {
            Some(&self.graph[mutation_root])
        } else {
            None
        }
    }

    pub fn root_subscription_node(&self) -> Option<&Node> {
        if let Some(subscription_root) = self.subscription_root {
            Some(&self.graph[subscription_root])
        } else {
            None
        }
    }

    pub fn edges_to(&self, node_index: NodeIndex) -> Edges<'_, Edge, Directed> {
        self.graph.edges_directed(node_index, Direction::Incoming)
    }

    pub fn edges_from(&self, node_index: NodeIndex) -> Edges<'_, Edge, Directed> {
        self.graph.edges_directed(node_index, Direction::Outgoing)
    }

    #[instrument(level = "trace", skip(self, state))]
    fn link_root_edges(&mut self, state: &SupergraphState) -> Result<(), GraphError> {
        for (def_name, definition) in state.definitions.iter() {
            if let Some(root_type) = definition.try_into_root_type() {
                for graph_id in definition.subgraphs().iter() {
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

                        let tail = self.upsert_node(Node::new_root_node(
                            def_name,
                            state.resolve_graph_id(graph_id)?,
                        ));

                        self.upsert_edge(
                            head,
                            tail,
                            Edge::SubgraphEntrypoint {
                                name: state.resolve_graph_id(graph_id)?,
                                operation_kind: root_type.clone(),
                            },
                        );
                    }
                }
            }
        }

        Ok(())
    }

    /// Returns the `@provides` of each field edge that has one, for `build_provides_edges`.
    #[instrument(level = "trace", skip(self, state))]
    fn build_field_edges(
        &mut self,
        state: &SupergraphState,
    ) -> Result<HashMap<EdgeIndex, Provided>, GraphError> {
        let unions = UnionDefinitions::new(state);
        let mut provides = HashMap::new();

        for (def_name, definition) in state.definitions.iter() {
            for graph_id in definition.subgraphs().iter() {
                let graph_name = state.resolve_graph_id(graph_id)?;
                if !definition.is_defined_in_subgraph(graph_id) {
                    continue;
                }

                let is_interface_object = definition
                    .extract_join_types_for(graph_id)
                    .iter()
                    .any(|j| j.is_interface_object);
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
                    let head = self.upsert_node(Node::new_node(
                        def_name,
                        state.resolve_graph_id(graph_id)?,
                        // __typename is not resolable for @interfaceObject so it's not it
                        false,
                    ));
                    let tail = self.upsert_node(Node::new_node(
                        "String",
                        state.resolve_graph_id(graph_id)?,
                        // String is not an @interfaceObject
                        false,
                    ));

                    self.upsert_typename_edge(head, tail, def_name);
                }

                trace!(
                    "[x] Creating self-referencing edge for '{}/{}'",
                    def_name,
                    graph_id
                );
                let head = self.upsert_node(Node::new_node(
                    def_name,
                    state.resolve_graph_id(graph_id)?,
                    state.is_interface_object_in_subgraph(def_name, graph_id),
                ));
                self.upsert_edge(head, head, Edge::Selfie(def_name.clone()));

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
                          (requires_str, join_field.graph_id.as_ref().expect("join__field(graph:) should exist when join__field(requires:) exists"))
                        })
                    }).map(|(requires_str, graph_id)| TypeAwareSelection {
                              type_name: def_name.to_string(),
                              selection_set: FederationRules::parse_requires(
                                state,
                                graph_id,
                                def_name,
                                requires_str,
                              )
                              .into(),
                          });

                    // Parsed once, a union field has an edge per member.
                    let provided = maybe_join_field
                        .and_then(|join_field| {
                            let graph_id = join_field.graph_id.as_ref()?;
                            FederationRules::parse_provides(
                                state,
                                join_field,
                                graph_id,
                                target_type,
                            )
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
                        let head = self.upsert_node(Node::new_node(
                            def_name,
                            state.resolve_graph_id(graph_id)?,
                            state.is_interface_object_in_subgraph(def_name, graph_id),
                        ));

                        // Build union-member edges for the current subgraph only. Doing a global
                        // intersection here strips valid members from pinned paths such as
                        // Query.getResponse -> Response/A -> actions, where A knows the full union.
                        let mut member_types = unions
                            .members_for_field_in_graph(field_definition, target_type, graph_id)
                            .into_iter()
                            .collect::<Vec<_>>();
                        member_types.sort_unstable();
                        let possible_members: Vec<String> = member_types
                            .iter()
                            .map(|member| member.to_string())
                            .collect::<Vec<_>>();

                        trace!(
                            "Handling a field {}.{}/{} resolving a union type {}",
                            def_name,
                            field_name,
                            graph_id,
                            target_type
                        );

                        for member in member_types {
                            let tail = self.upsert_node(Node::new_specialized_node(
                                target_type,
                                state.resolve_graph_id(graph_id)?,
                                state.is_interface_object_in_subgraph(target_type, graph_id),
                                SubgraphTypeSpecialization::UnionMembers(UnionMembersData {
                                    type_name: def_name.clone(),
                                    field_name: field_name.clone(),
                                    object_type_name: member.to_string(),
                                    possible_members: possible_members.clone(),
                                }),
                            ));
                            let abstract_tail = self.upsert_node(Node::new_node(
                                member,
                                state.resolve_graph_id(graph_id)?,
                                state.is_interface_object_in_subgraph(member, graph_id),
                            ));
                            // because we duplicate tails, we need to add __typename to all of them
                            let typename_tail = self.upsert_node(Node::new_node(
                                "String",
                                state.resolve_graph_id(graph_id)?,
                                false,
                            ));

                            trace!(
                                "  [x] Creating field move edge '{}.__typename/{}' (type: String)",
                                def_name,
                                graph_id
                            );
                            self.upsert_edge(
                                tail,
                                typename_tail,
                                Edge::create_field_move(
                                    "__typename".to_string(),
                                    target_type.to_string(),
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
                            let edge = self.upsert_edge(
                                head,
                                tail,
                                Edge::create_field_move(
                                    field_name.clone(),
                                    def_name.clone(),
                                    state.is_scalar_type(target_type),
                                    field_definition.field_type.is_list(),
                                    maybe_join_field.cloned(),
                                    requirements.clone(),
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
                            self.upsert_edge(
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

                    let current_subgraph_resolved_id = state.resolve_graph_id(graph_id)?;

                    let head = self.upsert_node(Node::new_node(
                        def_name,
                        current_subgraph_resolved_id.clone(),
                        state.is_interface_object_in_subgraph(def_name, graph_id),
                    ));
                    let tail = self.upsert_node(Node::new_node(
                        target_type,
                        current_subgraph_resolved_id.clone(),
                        state.is_interface_object_in_subgraph(target_type, graph_id),
                    ));

                    let edge = self.upsert_edge(
                        head,
                        tail,
                        Edge::create_field_move(
                            field_name.clone(),
                            def_name.clone(),
                            state.is_scalar_type(target_type),
                            field_definition.field_type.is_list(),
                            maybe_join_field.cloned(),
                            requirements.clone(),
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

                            if graph_id == current_subgraph_resolved_id.0 {
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
                                    field_definition.field_type.is_list(),
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

impl Graph {
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
        state: &SupergraphState,
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
        ctx: &mut ProvidesContext,
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
        let edges: Vec<(EdgeIndex, NodeIndex, Edge)> = self
            .graph
            .edges(original)
            .map(|edge| (edge.id(), edge.target(), edge.weight().clone()))
            .collect();
        for (edge_index, target, edge) in edges {
            let target = match &edge {
                Edge::FieldMove(field_move) => {
                    let mut below = provided
                        .fields
                        .get(&field_move.name)
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
                    field_name.clone(),
                    type_name.clone(),
                    ctx.state.is_scalar_type(return_type_name),
                    field_definition.field_type.is_list(),
                    None,
                    None,
                    None,
                ),
            );
        }
        for (on_type, below) in &provided.on_types {
            if self.member_targets(original, on_type).next().is_some() {
                continue;
            }
            let plain = self.upsert_node(Node::new_node(
                on_type,
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
                let below = match self.node_display_name_to_index.get(&plain.display_name()) {
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
        Ok(self
            .node_display_name_to_index
            .get(&plain.display_name())
            .copied())
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
impl Display for Graph {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", Dot::with_config(&self.graph, &[]))
    }
}
