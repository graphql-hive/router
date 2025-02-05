use std::env;
use std::process;

use query_planner::discovery_graph::DiscoveryGraph;
use query_planner::operation_advisor::OperationAdvisor;
use query_planner::parse_schema;
use query_planner::supergraph_metadata::SupergraphMetadata;

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() != 3 {
        eprintln!("Usage: query-planner <command> <supergraph_path>");
        process::exit(1);
    }

    match args[1].as_str() {
        "consumer_schema" => process_consumer_schema(&args[2]),
        "satisfiability_graph" => process_satisfiability_graph(&args[2]),
        "discovery_graph" => process_discovery_graph(&args[2]),
        _ => {
            eprintln!("Unknown command. Available commands: consumer_graph, satisfiability_graph");
            process::exit(1);
        }
    }
}

fn process_consumer_schema(path: &str) {
    let supergraph_sdl = std::fs::read_to_string(path).expect("Unable to read input file");
    let parsed_schema = parse_schema(&supergraph_sdl);
    let advisor = OperationAdvisor::new(SupergraphMetadata::new(&parsed_schema));
    println!("{}", advisor.consumer_schema.document);
}

fn process_discovery_graph(path: &str) {
    let supergraph_sdl = std::fs::read_to_string(path).expect("Unable to read input file");
    let parsed_schema = parse_schema(&supergraph_sdl);
    let discovery_graph = DiscoveryGraph::new_from_supergraph_metadata(&parsed_schema);
    println!("{}", discovery_graph);
}

fn process_satisfiability_graph(path: &str) {
    let supergraph_sdl = std::fs::read_to_string(path).expect("Unable to read input file");
    let parsed_schema = parse_schema(&supergraph_sdl);
    let advisor = OperationAdvisor::new(SupergraphMetadata::new(&parsed_schema));

    println!("{}", advisor.graph);
}
