pub mod plan_nodes;
pub mod resolution_path;
pub mod traversal_step;

use std::fmt::Debug;

use graphql_parser_hive_fork::query::OperationDefinition;
use petgraph::{graph::NodeIndex, visit::EdgeRef};
use resolution_path::ResolutionPath;
use tracing::{debug, instrument};
use traversal_step::Step;

use crate::{
    consumer_schema::ConsumerSchema,
    graph::{
        edge::{Edge, EdgePair},
        selection::SelectionNode,
        Graph,
    },
    state::supergraph_state::SupergraphState,
};

pub struct OperationAdvisor<'a> {
    pub supergraph_state: SupergraphState<'a>,
    pub graph: Graph,
    pub consumer_schema: ConsumerSchema,
}

impl Debug for OperationAdvisor<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OperationAdvisor").finish()
    }
}

#[derive(Debug)]
pub enum OperationType {
    Query,
    Mutation,
    Subscription,
}

impl From<&OperationDefinition<'static, String>> for OperationType {
    fn from(operation: &OperationDefinition<'static, String>) -> Self {
        match operation {
            OperationDefinition::Query(_) | OperationDefinition::SelectionSet(_) => {
                OperationType::Query
            }
            OperationDefinition::Mutation(_) => OperationType::Mutation,
            OperationDefinition::Subscription(_) => OperationType::Subscription,
        }
    }
}

impl<'a> OperationAdvisor<'a> {
    pub fn new(supergraph: SupergraphState<'a>) -> Self {
        let graph = Graph::new_from_supergraph(&supergraph);

        Self {
            consumer_schema: ConsumerSchema::new_from_supergraph(supergraph.document),
            supergraph_state: supergraph,
            graph,
        }
    }

    #[instrument(skip(self))]
    pub fn walk_steps(&self, op_type: &OperationType, steps: &Vec<Step>) {
        let root_entrypoints = self.get_entrypoints(op_type);
        let initial_paths = root_entrypoints
            .iter()
            .map(|node_index| ResolutionPath::new(*node_index))
            .collect::<Vec<ResolutionPath>>();

        steps.iter().fold(initial_paths, |current_paths, step| {
            if current_paths.is_empty() {
                return vec![];
            }

            let next_paths: Vec<ResolutionPath> = current_paths
                .iter()
                .flat_map(|path| {
                    debug!(
                        "looking for paths to step '{}' from node {:?}",
                        step.field_name(),
                        self.graph.node(path.root_node).id(),
                    );
                    let direct_paths = self.find_direct_paths(path, step);
                    debug!("found total of {} direct paths", direct_paths.len());
                    let indirect_paths = self.find_indirect_paths(path, step);
                    debug!("found total of {} indirect paths", indirect_paths.len());
                    let advance = !direct_paths.is_empty() || !indirect_paths.is_empty();
                    debug!("advance: {}", advance);

                    direct_paths.into_iter().chain(indirect_paths.into_iter())
                })
                .collect();

            next_paths
        });
    }

    #[instrument(skip(self))]
    fn find_direct_paths(&self, path: &ResolutionPath, step: &Step) -> Vec<ResolutionPath> {
        let mut result = vec![];
        let path_tail = path.tail(&self.graph);
        // Get all the edges from the current tail
        // Filter by FieldMove edges with matching field name and not already in path, to avoid loops
        let edges_iter = self
            .graph
            .edges_from(path_tail)
            .filter(|e| matches!(e.weight(), Edge::FieldMove { name, .. } if name == step.field_name() && !path.edges.contains(&e.id())));

        for edge in edges_iter {
            let edge_weight = edge.weight();
            let edge_id = &edge.id();
            let can_be_satisfied = self.can_satisfy_edge((edge_weight, *edge_id), path);

            match can_be_satisfied {
                Some(p) => {
                    debug!("edge satisfied: {:?}", p);
                    let next_resolution_path = path.advance_to(&self.graph, edge_id);
                    result.push(next_resolution_path);
                }
                None => {
                    debug!("edge not satisfied");
                }
            }
        }

        result
    }

    #[instrument(skip(self))]
    fn find_indirect_paths(&self, path: &ResolutionPath, step: &Step) -> Vec<ResolutionPath> {
        let tail_node_index = path.tail(&self.graph);
        let tail_node = self.graph.node(tail_node_index);
        let source_graph_id = tail_node.graph_id().expect("tail does not have graph info");
        println!("source_graph_id: {source_graph_id}");

        vec![]
    }

    #[instrument(skip(self))]
    fn can_satisfy_edge(
        &self,
        (edge, edge_id): EdgePair,
        path: &ResolutionPath,
    ) -> Option<Vec<ResolutionPath>> {
        debug!(edge_weight = debug(edge));

        match edge.requirements_selections() {
            None => {
                debug!("edge does not have requirements, will return empty array");
                Some(vec![])
            }
            Some(selections) => {
                debug!(
                    "checking requirements for '{:?}' in edge '{}'",
                    selections,
                    edge.id()
                );

                let mut requirements: Vec<MoveRequirement> = vec![];
                let paths_to_requirements: Vec<ResolutionPath> = vec![];

                for selection in selections.selection_set.iter() {
                    requirements.splice(
                        0..0,
                        vec![MoveRequirement {
                            paths: vec![path.clone()],
                            selection: selection.clone(),
                        }],
                    );
                }

                while let Some(requirement) = requirements.pop() {
                    // Process the requirement here
                    match &requirement.selection {
                        SelectionNode::Field {
                            field_name,
                            type_name,
                            selections,
                        } => {
                            let result =
                                validate_field_requirement(field_name, type_name, selections);
                            // Process the field selection here
                        }
                        SelectionNode::Fragment { .. } => {
                            unimplemented!("fragment not supported yet")
                        }
                    }
                }

                Some(vec![])
            }
        }
    }

    fn get_entrypoints(&self, operation_type: &OperationType) -> Vec<NodeIndex> {
        let entrypoint_root = match operation_type {
            OperationType::Query => self.graph.query_root,
            OperationType::Mutation => self.graph.mutation_root.unwrap(),
            OperationType::Subscription => self.graph.subscription_root.unwrap(),
        };

        self.graph
            .edges_from(entrypoint_root)
            .map(|e| e.target())
            .collect::<Vec<NodeIndex>>()
    }

    // fn root_selection_set(
    //     &'a self,
    //     operation: &'a OperationDefinition<'static, String>,
    // ) -> &'a SelectionSet<'static, String> {
    //     match operation {
    //         OperationDefinition::Query(q) => &q.selection_set,
    //         OperationDefinition::SelectionSet(s) => &s,
    //         OperationDefinition::Mutation(m) => &m.selection_set,
    //         OperationDefinition::Subscription(s) => &s.selection_set,
    //     }
    // }
}

pub struct MoveRequirement {
    pub paths: Vec<ResolutionPath>,
    pub selection: SelectionNode,
}
