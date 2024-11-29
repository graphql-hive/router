mod edge;
mod graph;
mod join_field;
mod join_implements;
mod join_type;
mod node;
mod supergraph;

use std::fs;

use graph::GraphQLSatisfiabilityGraph;

fn main() {
    let supergraph_sdl = fs::read_to_string("fixture/dotan.supergraph.graphql")
        .expect("Unable to read supergraph.graphql");

    // let supergraph = supergraph::parse_supergraph(&supergraph_sdl).unwrap();
    let graph = GraphQLSatisfiabilityGraph::new_from_supergraph_sdl(&supergraph_sdl);

    match graph {
        Ok(graph) => println!("{}", graph),
        Err(e) => eprintln!("Failed to build graph: {}", e),
    }
}
