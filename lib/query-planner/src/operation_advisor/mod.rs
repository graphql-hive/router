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

        let consumer_schema = ConsumerSchema::new_from_supergraph(supergraph.document);

        println!("consumer_schema = {}", consumer_schema.document);

        Self {
            supergraph_metadata: supergraph,
            consumer_schema,
            graph,
        }
    }

    #[cfg(test)]
    pub fn print_graph(&self) -> String {
        format!("{}", self.graph)
    }
}
