use graphql_parser_hive_fork::query::OperationDefinition;

use crate::{
    consumer_schema::ConsumerSchema, graph::GraphQLSatisfiabilityGraph,
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

    pub fn travel_plan(&self, _operation: OperationDefinition<'static, String>) {
        unimplemented!("travel_plan")
    }
}
