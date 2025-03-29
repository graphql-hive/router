use std::env;
use std::process;

use graphql_parser_hive_fork::query::Definition;
use query_planner::operation_advisor::OperationAdvisor;
use query_planner::parse_operation;
use query_planner::parse_schema;
use query_planner::supergraph_metadata::SupergraphMetadata;

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() < 3 {
        eprintln!("Usage: query-planner <command> <supergraph_path>");
        process::exit(1);
    }

    match args[1].as_str() {
        "consumer_schema" => process_consumer_schema(&args[2]),
        "satisfiability_graph" => process_satisfiability_graph(&args[2]),
        "travel_plan" => process_travel_plan(&args[2], &args[3]),
        _ => {
            eprintln!("Unknown command. Available commands: consumer_graph, satisfiability_graph, travel_plan");
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

fn process_satisfiability_graph(path: &str) {
    let supergraph_sdl = std::fs::read_to_string(path).expect("Unable to read input file");
    let parsed_schema = parse_schema(&supergraph_sdl);
    let advisor = OperationAdvisor::new(SupergraphMetadata::new(&parsed_schema));

    println!("{}", advisor.graph);
}

fn process_travel_plan(supergraph_path: &str, operation_path: &str) {
    let supergraph_sdl =
        std::fs::read_to_string(supergraph_path).expect("Unable to read input file");
    let operation_text =
        std::fs::read_to_string(operation_path).expect("Unable to read input file");
    let parsed_schema = parse_schema(&supergraph_sdl);
    let advisor = OperationAdvisor::new(SupergraphMetadata::new(&parsed_schema));

    let operation = parse_operation(&operation_text);

    advisor.travel_plan(match &operation.definitions[0] {
        Definition::Operation(operation) => operation.clone(),
        _ => panic!("Expected operation definition"),
    });

    println!("{:?}", ());
}
