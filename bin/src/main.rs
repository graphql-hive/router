use std::env;
use std::process;

use query_planner::parse_schema;
use query_planner::planner::traversal_step::Step;
use query_planner::planner::Planner;
use query_planner::state::supergraph_state::RootOperationType;
use query_planner::state::supergraph_state::SupergraphState;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

fn main() {
    let tree_layer = tracing_tree::HierarchicalLayer::new(3)
        // open/close logging
        .with_span_modes(false)
        // timer
        .with_timer(tracing_tree::time::Uptime::default())
        // unused
        .with_thread_names(false)
        .with_thread_ids(false)
        .with_targets(false);

    tracing_subscriber::registry().with(tree_layer).init();

    let args: Vec<String> = env::args().collect();

    if args.len() < 3 {
        eprintln!("Usage: query-planner <command> <supergraph_path> [...]");
        process::exit(1);
    }

    match args[1].as_str() {
        "consumer_schema" => process_consumer_schema(&args[2]),
        "graph" => process_graph(&args[2]),
        "paths" => process_paths(&args[2], &args[3]),
        // "plan" => process_plan(&args[2], &args[3]),
        _ => {
            eprintln!("Unknown command. Available commands: consumer_graph, graph, paths");
            process::exit(1);
        }
    }
}

fn process_consumer_schema(path: &str) {
    let supergraph_sdl = std::fs::read_to_string(path).expect("Unable to read input file");
    let parsed_schema = parse_schema(&supergraph_sdl);
    let advisor =
        Planner::new(SupergraphState::new(&parsed_schema)).expect("failed to build planner");
    println!("{}", advisor.consumer_schema.document);
}

fn process_graph(path: &str) {
    let supergraph_sdl = std::fs::read_to_string(path).expect("Unable to read input file");
    let parsed_schema = parse_schema(&supergraph_sdl);
    let advisor =
        Planner::new(SupergraphState::new(&parsed_schema)).expect("failed to build planner");

    println!("{}", advisor.graph);
}

fn process_paths(supergraph_path: &str, steps: &str) {
    let supergraph_sdl =
        std::fs::read_to_string(supergraph_path).expect("Unable to read input file");
    let parsed_schema = parse_schema(&supergraph_sdl);
    let advisor =
        Planner::new(SupergraphState::new(&parsed_schema)).expect("failed to build planner");
    let steps = Step::parse_field_step(steps);

    advisor
        .walk_steps(&RootOperationType::Query, &steps)
        .expect("failed to walk");
}

// fn process_plan(supergraph_path: &str, operation_path: &str) {
//     let supergraph_sdl =
//         std::fs::read_to_string(supergraph_path).expect("Unable to read input file");
//     let operation_text =
//         std::fs::read_to_string(operation_path).expect("Unable to read input file");
//     let parsed_schema = parse_schema(&supergraph_sdl);
//     let advisor = OperationAdvisor::new(SupergraphState::new(&parsed_schema));

//     let operation = parse_operation(&operation_text);

//     advisor.travel_plan(match &operation.definitions[0] {
//         Definition::Operation(operation) => operation.clone(),
//         _ => panic!("Expected operation definition"),
//     });
// }
