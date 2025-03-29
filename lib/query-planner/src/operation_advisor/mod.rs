mod pathfinder;
mod traversal;

use graphql_parser_hive_fork::query::OperationDefinition;
use pathfinder::Pathfinder;

use crate::{
    consumer_schema::ConsumerSchema, satisfiability_graph::graph::GraphQLSatisfiabilityGraph,
    supergraph_metadata::SupergraphMetadata,
};

pub struct OperationAdvisor<'a> {
    pub supergraph_metadata: SupergraphMetadata<'a>,
    pub graph: GraphQLSatisfiabilityGraph,
    pub consumer_schema: ConsumerSchema,
}

impl<'a> OperationAdvisor<'a> {
    pub fn new(supergraph: SupergraphMetadata<'a>) -> Self {
        let graph = GraphQLSatisfiabilityGraph::new_from_supergraph(&supergraph)
            .expect("failed to build graph");

        Self {
            consumer_schema: ConsumerSchema::new_from_supergraph(supergraph.document),
            supergraph_metadata: supergraph,
            graph,
        }
    }

    pub fn travel_plan(&self, operation: OperationDefinition<'static, String>) {
        let deps_tree = Pathfinder::new(&self.graph).find_paths_for_operation(&operation);

        println!("deps_tree: {:#?}", deps_tree);
    }
}
