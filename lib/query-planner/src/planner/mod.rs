pub mod plan_nodes;
pub mod resolution_path;
pub mod traversal_step;

use std::fmt::Debug;

use petgraph::{graph::NodeIndex, visit::EdgeRef};
use resolution_path::ResolutionPath;
use tracing::{debug, instrument};
use traversal_step::Step;

use crate::{
    consumer_schema::ConsumerSchema,
    graph::{
        edge::{Edge, EdgePair},
        error::GraphError,
        selection::SelectionNode,
        Graph,
    },
    state::supergraph_state::{RootOperationType, SupergraphState},
};

pub struct Planner<'a> {
    pub supergraph_state: SupergraphState<'a>,
    pub graph: Graph,
    pub consumer_schema: ConsumerSchema,
}

impl Debug for Planner<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Planner").finish()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PlannerError {
    #[error("Graph construction error: {0}")]
    GraphError(GraphError),
    #[error("Tail {0:?} is missing information")]
    TailMissingInfo(NodeIndex),
}

impl From<GraphError> for PlannerError {
    fn from(error: GraphError) -> Self {
        PlannerError::GraphError(error)
    }
}

impl<'a> Planner<'a> {
    pub fn new(supergraph: SupergraphState<'a>) -> Result<Self, PlannerError> {
        let graph = Graph::graph_from_supergraph_state(&supergraph)?;

        Ok(Self {
            consumer_schema: ConsumerSchema::new_from_supergraph(supergraph.document),
            supergraph_state: supergraph,
            graph,
        })
    }

    #[instrument(skip(self))]
    pub fn walk_steps(
        &self,
        op_type: &RootOperationType,
        steps: &Vec<Step>,
    ) -> Result<(), PlannerError> {
        let root_entrypoints = self.get_entrypoints(op_type)?;
        let initial_paths = root_entrypoints
            .iter()
            .map(|node_index| ResolutionPath::new(*node_index))
            .collect::<Vec<ResolutionPath>>();

        steps
            .iter()
            .try_fold(initial_paths, |current_paths, step| {
                if current_paths.is_empty() {
                    return Ok(vec![]);
                }

                let next_paths: Result<Vec<ResolutionPath>, PlannerError> =
                    current_paths.iter().try_fold(vec![], |mut acc, path| {
                        debug!(
                            "looking for paths to step '{}' from node {:?}",
                            step.field_name(),
                            self.graph.node(path.root_node).unwrap().id(),
                        );
                        let direct_paths = self.find_direct_paths(path, step)?;
                        debug!("found total of {} direct paths", direct_paths.len());
                        let indirect_paths = self.find_indirect_paths(path, step)?;
                        debug!("found total of {} indirect paths", indirect_paths.len());
                        let advance = !direct_paths.is_empty() || !indirect_paths.is_empty();
                        debug!("advance: {}", advance);

                        acc.extend(direct_paths.into_iter());
                        acc.extend(indirect_paths.into_iter());

                        Ok(acc)
                    });

                next_paths
            })?;

        Ok(())
    }

    #[instrument(skip(self))]
    fn find_direct_paths(
        &self,
        path: &ResolutionPath,
        step: &Step,
    ) -> Result<Vec<ResolutionPath>, PlannerError> {
        todo!("Implement find_direct_paths and others");
        let mut result: Vec<ResolutionPath> = vec![];
        let path_tail = path.tail(&self.graph)?;

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
                    let next_resolution_path = path.advance_to(&self.graph, edge_id)?;
                    result.push(next_resolution_path);
                }
                None => {
                    debug!("edge not satisfied");
                }
            }
        }

        Ok(result)
    }

    #[instrument(skip(self))]
    fn find_indirect_paths(
        &self,
        path: &ResolutionPath,
        step: &Step,
    ) -> Result<Vec<ResolutionPath>, PlannerError> {
        let tail_node_index = path.tail(&self.graph)?;
        let tail_node = self.graph.node(tail_node_index)?;
        let source_graph_id = tail_node
            .graph_id()
            .ok_or_else(|| PlannerError::TailMissingInfo(tail_node_index))?;
        println!("source_graph_id: {source_graph_id}");

        Ok(vec![])
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

                // it's important to pop from the end as we want to process the last added requirement first
                while let Some(requirement) = requirements.pop() {
                    match &requirement.selection {
                        SelectionNode::Field {
                            field_name,
                            type_name,
                            selections,
                        } => {
                            // let result =
                            // validate_field_requirement(field_name, type_name, selections);

                            // match result {}
                        }
                        SelectionNode::Fragment { .. } => {
                            unimplemented!("fragment not supported yet")
                        }
                    }
                }

                Some(paths_to_requirements)
            }
        }
    }

    fn get_entrypoints(
        &self,
        operation_type: &RootOperationType,
    ) -> Result<Vec<NodeIndex>, PlannerError> {
        let entrypoint_root = match operation_type {
            RootOperationType::Query => Some(self.graph.query_root),
            RootOperationType::Mutation => self.graph.mutation_root,
            RootOperationType::Subscription => self.graph.subscription_root,
        }
        .ok_or_else(|| GraphError::MissingRootType(*operation_type))?;

        Ok(self
            .graph
            .edges_from(entrypoint_root)
            .map(|e| e.target())
            .collect::<Vec<NodeIndex>>())
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
