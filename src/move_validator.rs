use graphql_parser_hive_fork::query::SelectionSet;
use petgraph::graph::NodeIndex;

use crate::graph::GraphQLSatisfiabilityGraph;

pub struct MoveValidator {
    graph: GraphQLSatisfiabilityGraph,
}

impl MoveValidator {
    pub fn new(graph: GraphQLSatisfiabilityGraph) -> MoveValidator {
        MoveValidator { graph }
    }

    fn validate_move_from_root(
        &self,
        root_node: NodeIndex,
        selection_set: &SelectionSet<'static, String>,
    ) {
        todo!("validate_move_from_root")
    }
}
