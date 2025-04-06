use graphql_parser_hive_fork::query::OperationDefinition;

use crate::{consumer_schema::ConsumerSchema, graph::Graph, supergraph_metadata::SupergraphState};

pub struct OperationAdvisor<'a> {
    pub supergraph_metadata: SupergraphState<'a>,
    pub graph: Graph,
    pub consumer_schema: ConsumerSchema,
}

impl<'a> OperationAdvisor<'a> {
    pub fn new(supergraph: SupergraphState<'a>) -> Self {
        let graph = Graph::new_from_supergraph(&supergraph);

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
