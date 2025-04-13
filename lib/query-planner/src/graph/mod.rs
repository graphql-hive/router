pub mod edge;
pub mod error;
pub mod node;
pub mod selection;

mod tests;

use std::{
    collections::HashMap,
    fmt::{Debug, Display},
};

use crate::{
    federation_spec::FederationRules,
    state::supergraph_state::{RootOperationType, SupergraphDefinition, SupergraphState},
};
use error::GraphError;
use graphql_parser_hive_fork::query::{Selection, SelectionSet};
use graphql_tools::ast::{SchemaDocumentExtension, TypeExtension};
use node::SubgraphType;
use petgraph::{
    dot::Dot,
    graph::{EdgeIndex, Edges, NodeIndex},
    visit::EdgeRef,
    Directed, Direction, Graph as Petgraph,
};
use tracing::{debug, info, instrument};

use super::graph::{edge::Edge, node::Node};

type InnerGraph = Petgraph<Node, Edge, Directed>;

#[derive(Debug, Default)]
pub struct Graph {
    pub graph: InnerGraph,
    pub query_root: NodeIndex,
    pub mutation_root: Option<NodeIndex>,
    pub subscription_root: Option<NodeIndex>,
    pub node_to_index: HashMap<String, NodeIndex>,
}

impl Graph {
    #[instrument(skip(supergraph_state))]
    pub fn graph_from_supergraph_state(
        supergraph_state: &SupergraphState,
    ) -> Result<Self, GraphError> {
        let mut instance = Graph {
            node_to_index: HashMap::new(),
            graph: InnerGraph::new(),
            ..Default::default()
        };

        instance.build_graph(supergraph_state)?;

        Ok(instance)
    }

    pub fn node(&self, node_index: NodeIndex) -> Result<&Node, GraphError> {
        self.graph
            .node_weight(node_index)
            .ok_or_else(|| GraphError::NodeNotFound(node_index))
    }

    pub fn edge(&self, edge_index: EdgeIndex) -> Result<&Edge, GraphError> {
        self.graph
            .edge_weight(edge_index)
            .ok_or_else(|| GraphError::EdgeNotFound(edge_index))
    }

    pub fn get_edge_tail(&self, edge_index: &EdgeIndex) -> Result<NodeIndex, GraphError> {
        self.graph
            .edge_endpoints(*edge_index)
            .ok_or_else(|| GraphError::EdgeNotFound(*edge_index))
            .map(|v| v.1)
    }

    #[instrument(skip(self, state))]
    fn build_graph(&mut self, state: &SupergraphState) -> Result<(), GraphError> {
        debug!(
            "Building graph for supergraph with {} definitions",
            state.document.definitions.len()
        );

        self.build_root_nodes(state)?;
        self.link_root_edges(state)?;
        self.build_field_edges(state)?;
        self.build_interface_implementation_edges(state)?;
        self.build_entity_reference_edges(state)?;
        self.build_viewed_field_edges(state)?;

        Ok(())
    }

    #[instrument(skip(self, state))]
    fn build_root_nodes(&mut self, state: &SupergraphState<'_>) -> Result<(), GraphError> {
        self.query_root =
            self.upsert_node(Node::QueryRoot(state.document.query_type().name.clone()));
        debug!(
            "added root type for queries: {}",
            state.document.query_type().name
        );
        self.mutation_root = state.document.mutation_type().map(|mutation_type| {
            debug!("added root type for mutations: {}", mutation_type.name);
            self.upsert_node(Node::MutationRoot(mutation_type.name.clone()))
        });
        self.subscription_root = state.document.subscription_type().map(|subscription_type| {
            debug!(
                "added root type for subscriptions: {}",
                subscription_type.name
            );
            self.upsert_node(Node::SubscriptionRoot(subscription_type.name.clone()))
        });

        Ok(())
    }

    pub fn upsert_node(&mut self, node: Node) -> NodeIndex {
        let id = node.id();

        if let Some(index) = self.node_to_index.get(&id) {
            return *index;
        }

        let index = self.graph.add_node(node);
        self.node_to_index.insert(id, index);
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

    #[instrument(skip(self, state))]
    fn build_entity_reference_edges(
        &mut self,
        state: &SupergraphState<'_>,
    ) -> Result<(), GraphError> {
        for (def_name, definition) in state.definitions.iter() {
            for join_type1 in definition.join_types() {
                for join_type2 in definition.join_types() {
                    let head =
                        self.upsert_node(Node::subgraph_type(def_name, &join_type1.graph_id));

                    if join_type1.graph_id != join_type2.graph_id {
                        if let Some(key) = &join_type2.key {
                            let tail = self
                                .upsert_node(Node::subgraph_type(def_name, &join_type2.graph_id));
                            let selection_resolver =
                                state.selection_resolvers_for_subgraph(&join_type2.graph_id);
                            let selection = selection_resolver.resolve(def_name, key);

                            info!(
                                "Creating entity move edge from '{}/{}' to '{}/{}' via key '{}'",
                                def_name, join_type1.graph_id, def_name, join_type2.graph_id, key
                            );

                            self.upsert_edge(head, tail, Edge::create_entity_move(key, selection));
                        }
                    } else if let Some(key) = &join_type1.key {
                        let selection_resolver =
                            state.selection_resolvers_for_subgraph(&join_type1.graph_id);
                        let selection = selection_resolver.resolve(def_name, key);

                        info!(
                            "Creating self-referencing entity move edge in '{}/{}' via key '{}'",
                            def_name, join_type1.graph_id, key
                        );

                        self.upsert_edge(head, head, Edge::create_entity_move(key, selection));
                    }
                }
            }
        }

        Ok(())
    }

    #[instrument(skip(self, state))]
    fn build_interface_implementation_edges(
        &mut self,
        state: &SupergraphState<'_>,
    ) -> Result<(), GraphError> {
        for (def_name, definition) in state
            .definitions
            .iter()
            .filter(|(_, d)| matches!(d, SupergraphDefinition::Object(_)))
        {
            for join_implements in definition.join_implements() {
                let tail =
                    self.upsert_node(Node::subgraph_type(def_name, &join_implements.graph_id));
                let head = self.upsert_node(Node::subgraph_type(
                    &join_implements.interface,
                    &join_implements.graph_id,
                ));

                info!(
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

    #[instrument(skip(self, state))]
    fn link_root_edges(&mut self, state: &SupergraphState<'_>) -> Result<(), GraphError> {
        for (def_name, definition) in state.definitions.iter() {
            if let Some(root_type) = definition.try_into_root_type() {
                for graph_id in definition.subgraphs().iter() {
                    for (field_name, field_definition) in definition.fields() {
                        let (is_available, _) = FederationRules::check_field_subgraph_availability(
                            field_definition,
                            graph_id,
                            definition,
                        );

                        if !is_available {
                            continue;
                        }

                        let head = match root_type {
                            RootOperationType::Query => Some(self.query_root),
                            RootOperationType::Mutation => self.mutation_root,
                            RootOperationType::Subscription => self.subscription_root,
                        }
                        .ok_or_else(|| GraphError::MissingRootType(*root_type))?;

                        let tail = self.upsert_node(Node::subgraph_type(def_name, graph_id));

                        self.upsert_edge(
                            head,
                            tail,
                            Edge::RootEntrypoint {
                                field_name: field_name.clone(),
                            },
                        );
                    }
                }
            }
        }

        Ok(())
    }

    #[instrument(skip(self, state))]
    fn build_field_edges(&mut self, state: &SupergraphState<'_>) -> Result<(), GraphError> {
        for (def_name, definition) in state.definitions.iter() {
            for graph_id in definition.subgraphs().iter() {
                if definition.is_defined_in_subgraph(graph_id) {
                    for (field_name, field_definition) in definition.fields().iter() {
                        let (is_available, maybe_join_field) =
                            FederationRules::check_field_subgraph_availability(
                                field_definition,
                                graph_id,
                                definition,
                            );

                        let target_type = field_definition.source.field_type.inner_type();

                        match (is_available, maybe_join_field) {
                            (true, Some(join_field)) => {
                                let is_external = join_field.external.is_some_and(|v| v)
                                    && join_field.requires.is_none();
                                let has_provides = join_field.provides.is_some();

                                if is_external || has_provides {
                                    info!(
                                        "[ ] Field '{}.{}/{}' is external or has provides, skipping edge creation",
                                        def_name, field_name, graph_id
                                    );

                                    continue;
                                }

                                let head =
                                    self.upsert_node(Node::subgraph_type(def_name, graph_id));
                                let tail =
                                    self.upsert_node(Node::subgraph_type(target_type, graph_id));

                                info!(
                                    "[x] Creating owned field move edge '{}.{}/{}' (type: {})",
                                    def_name, field_name, graph_id, target_type
                                );

                                self.upsert_edge(
                                    head,
                                    tail,
                                    Edge::create_field_move(
                                        field_name.clone(),
                                        Some(join_field.clone()),
                                    ),
                                );
                            }
                            (true, None) => {
                                let head =
                                    self.upsert_node(Node::subgraph_type(def_name, graph_id));
                                let tail =
                                    self.upsert_node(Node::subgraph_type(target_type, graph_id));

                                info!(
                                    "[x] Creating field move edge for '{}.{}/{}' (type: {})",
                                    def_name, field_name, graph_id, target_type
                                );

                                self.upsert_edge(
                                    head,
                                    tail,
                                    Edge::create_field_move(field_name.clone(), None),
                                );
                            }
                            // The field is not available in the current subgraph
                            _ => {
                                info!(
                                    "[ ] Field '{}.{}/{}' does is not available in the subgraph, skipping edge creation (type: {})",
                                    def_name, field_name, graph_id, target_type
                                );
                            }
                        };
                    }
                }
            }
        }

        Ok(())
    }

    fn handle_viewed_selection_set(
        &mut self,
        state: &SupergraphState,
        selection_set: &SelectionSet<'static, String>,
        graph_id: &str,
        parent_type_def: &SupergraphDefinition<'_>,
        head: NodeIndex,
        view_id: u64,
    ) -> Result<(), GraphError> {
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
                    let return_type_name = field_in_parent.source.field_type.inner_type();

                    let subgraph_type = SubgraphType {
                        name: return_type_name.to_string(),
                        subgraph: graph_id.to_string(),
                    };

                    info!(
                        "Upserting graph viewed node for '{}.{}'",
                        return_type_name, graph_id,
                    );

                    let tail = self.upsert_node(match is_leaf {
                        true => Node::SubgraphType(subgraph_type),
                        false => Node::SubgraphTypeView {
                            view_id,
                            node: subgraph_type,
                            selection_set: field.selection_set.to_string(),
                            // selection_set: SelectionNode::parse_field_selection(
                            //     field.selection_set.to_string(),
                            //     &field.name,
                            //     return_type_name,
                            // ),
                        },
                    });

                    info!(
                        "Creating viewed (#{}) field edge for '{}.{}' (type: {})",
                        view_id,
                        parent_type_def.name(),
                        field.name,
                        return_type_name
                    );

                    self.upsert_edge(
                        head,
                        tail,
                        Edge::create_field_move(field.name.to_string(), None),
                    );

                    if !is_leaf {
                        let return_type =
                            state.definitions.get(return_type_name).ok_or_else(|| {
                                GraphError::DefinitionNotFound(return_type_name.to_string())
                            })?;

                        self.handle_viewed_selection_set(
                            state,
                            &field.selection_set,
                            graph_id,
                            return_type,
                            tail,
                            view_id,
                        )?;
                    }
                }
                _ => unimplemented!("fragments are not supported in provides yet"),
            };
        }

        Ok(())
    }

    #[instrument(skip(self, state))]
    fn build_viewed_field_edges(&mut self, state: &SupergraphState) -> Result<(), GraphError> {
        for (def_name, definition) in state.definitions.iter() {
            for join_type in definition.join_types().iter() {
                let mut view_id = 0;

                for (field_name, field_definition) in definition.fields().iter() {
                    for join_field in field_definition.join_field.iter() {
                        if join_field
                            .graph_id
                            .as_ref()
                            .is_some_and(|v| v == &join_type.graph_id)
                            && join_field.provides.is_some()
                        {
                            if let Some(selection_set) = FederationRules::parse_provides(join_field)
                            {
                                view_id += 1;

                                let head = self.upsert_node(Node::subgraph_type(
                                    definition.name(),
                                    &join_type.graph_id,
                                ));

                                let return_type_name =
                                    field_definition.source.field_type.inner_type();

                                let tail = self.upsert_node(Node::SubgraphTypeView {
                                    view_id,
                                    node: SubgraphType {
                                        name: return_type_name.to_string(),
                                        subgraph: join_type.graph_id.to_string(),
                                    },
                                    selection_set: selection_set.to_string(),
                                    // selection_set: SelectionNode::parse_field_selection(
                                    //     selection_set.to_string(),
                                    //     &field_name,
                                    //     return_type_name,
                                    // ),
                                });

                                info!(
                                    "Creating viewed (#{}) link for provided field '{}.{}/{:?}' (type: {})",
                                    view_id, def_name, field_name, join_type.graph_id, return_type_name
                                );

                                self.upsert_edge(
                                    head,
                                    tail,
                                    Edge::create_field_move(
                                        field_name.to_string(),
                                        Some(join_field.clone()),
                                    ),
                                );

                                let return_type =
                                    state.definitions.get(return_type_name).ok_or_else(|| {
                                        GraphError::DefinitionNotFound(return_type_name.to_string())
                                    })?;

                                self.handle_viewed_selection_set(
                                    state,
                                    &selection_set,
                                    &join_type.graph_id,
                                    return_type,
                                    tail,
                                    view_id,
                                )?;
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }
}

/// Print me with `println!("{}", graph);` to see the graph in DOT/digraph format.
impl Display for Graph {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", Dot::with_config(&self.graph, &[]))
    }
}
