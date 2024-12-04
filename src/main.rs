mod edge;
mod graph;
mod join_field;
mod join_implements;
mod join_type;
mod move_validator;
mod node;
mod supergraph;

use std::fs;

use graph::GraphQLSatisfiabilityGraph;
use graphql_parser_hive_fork::parse_query;
use move_validator::MoveValidator;

fn main() {
    let supergraph_sdl = fs::read_to_string("fixture/dotan.supergraph.graphql")
        .expect("Unable to read supergraph.graphql");

    let operation = fs::read_to_string("fixture/dotan.operation.graphql")
        .expect("Unable to read dotan.operation.graphql");

    let parsed_schema = graphql_parser_hive_fork::parse_schema(&supergraph_sdl)
        .unwrap()
        .into_static();
    let graph = GraphQLSatisfiabilityGraph::new_from_supergraph_sdl(&parsed_schema);

    match graph {
        Ok(graph) => {
            println!("Graph = {}", graph);
            let move_validator = MoveValidator::new(graph);
            let parsed_operation = parse_query(&operation).unwrap().into_static();
            // what's next?
            // let validate_move = move_validator.validate_operation_move(&parsed_operation, None);
            // println!("Move result = {:?}", validate_move);
        }
        Err(e) => eprintln!("Failed to build graph: {}", e),
    }
}
