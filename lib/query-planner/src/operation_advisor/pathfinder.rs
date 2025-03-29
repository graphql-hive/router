use graphql_parser_hive_fork::query::{Field, OperationDefinition, SelectionSet};
use graphql_tools::static_graphql::query::Selection;
use petgraph::graph::NodeIndex;

use crate::satisfiability_graph::edge::Edge;
use crate::satisfiability_graph::graph::GraphQLSatisfiabilityGraph;
use crate::satisfiability_graph::node::Node;

use super::traversal::{OperationTraversal, TraversalNode};

pub struct Pathfinder<'a> {
    graph: &'a GraphQLSatisfiabilityGraph,
}

impl<'a> Pathfinder<'a> {
    pub fn new(graph: &'a GraphQLSatisfiabilityGraph) -> Self {
        Pathfinder { graph }
    }

    pub fn find_paths_for_operation(
        &self,
        operation: &OperationDefinition<'static, String>,
    ) -> Vec<TraversalNode> {
        let traversal = OperationTraversal::new(operation, self.graph);
        let nodes = traversal.travel_graph();

        nodes
    }
}
