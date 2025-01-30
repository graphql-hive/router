use graphql_parser_hive_fork::query::Document;
use petgraph::visit::{depth_first_search, Control};
use thiserror::Error;

use crate::{
    graph::GraphQLSatisfiabilityGraph, move_validator::MoveValidator, supergraph::SupergraphIR,
};

pub struct OperationAdvisor<'a> {
    supergraph: SupergraphIR<'a>,
    graph: GraphQLSatisfiabilityGraph,
    move_validator: MoveValidator,
}

impl<'a> OperationAdvisor<'a> {
    pub fn new(supergraph: SupergraphIR<'a>) -> Self {
        let graph = GraphQLSatisfiabilityGraph::new_from_supergraph(&supergraph)
            .expect("failed to build graph");
        let move_validator = MoveValidator::new();

        Self {
            supergraph,
            graph,
            move_validator,
        }
    }

    pub fn validate(&self, operation: Document<'static, String>) -> Result<(), ValidationError> {
        let op_type: &graphql_parser_hive_fork::query::Definition<'_, String> = &operation.definitions[0];
        let root_index = self.graph.lookup.query_root;
        let result = depth_first_search(&self.graph.lookup.graph, Some(root_index), |m| {
            println!("m: {:?}", m);
            // self.graph.lookup

            Control::<()>::Continue
        });

        println!("result: {:?}", result);

        Err(ValidationError::Todo)
    }
}

#[derive(Debug, Error)]
pub enum ValidationError {
    #[error("TODO")]
    Todo,
}
