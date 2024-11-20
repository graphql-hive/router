mod graph;
pub mod supergraph;

use std::fs;

use graph::Graph;

fn main() {
    let supergraph_sdl = fs::read_to_string("fixture/supergraph.graphql")
        .expect("Unable to read supergraph.graphql");

    let supergraph = supergraph::parse_supergraph(&supergraph_sdl).unwrap();
    let mut super_graph = Graph::new(supergraph, "Supergraph".to_string());

    super_graph.add_from_roots();

    // println!("{:?}", super_graph);
}
