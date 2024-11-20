pub mod graph;
pub mod supergraph;

use std::fs;

fn main() {
    let supergraph_sdl = fs::read_to_string("fixture/supergraph.graphql")
        .expect("Unable to read supergraph.graphql");

    let supergraph = supergraph::parse_supergraph(&supergraph_sdl);

    println!("{:?}", supergraph);
}
