use std::env;
use std::process;

use query_planner::ast::normalization::normalize_operation;
use query_planner::ast::operation::OperationDefinition;
use query_planner::consumer_schema::ConsumerSchema;
use query_planner::graph::Graph;
use query_planner::planner::best::find_best_combination;
use query_planner::planner::fetch::fetch_graph::build_fetch_graph_from_query_tree;
use query_planner::planner::fetch::fetch_graph::FetchGraph;
use query_planner::planner::plan_nodes::QueryPlan;
use query_planner::planner::query_plan::build_query_plan_from_fetch_graph;
use query_planner::planner::tree::query_tree::QueryTree;
use query_planner::planner::walker::walk_operation;
use query_planner::planner::walker::ResolvedOperation;
use query_planner::state::supergraph_state::SupergraphState;
use query_planner::utils::parsing::parse_operation;
use query_planner::utils::parsing::parse_schema;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

fn main() {
    let tree_layer = tracing_tree::HierarchicalLayer::new(2)
        .with_bracketed_fields(true)
        .with_deferred_spans(false)
        .with_wraparound(25)
        .with_indent_lines(true)
        .with_timer(tracing_tree::time::Uptime::default())
        .with_thread_names(false)
        .with_thread_ids(false)
        .with_targets(false);

    tracing_subscriber::registry()
        .with(tree_layer)
        .with(EnvFilter::from_default_env())
        .init();

    let args: Vec<String> = env::args().collect();

    if args.len() < 3 {
        eprintln!("Usage: query-planner <command> <supergraph_path> [...]");
        process::exit(1);
    }

    match args[1].as_str() {
        "consumer_schema" => process_consumer_schema(&args[2]),
        "graph" => {
            let supergraph_sdl =
                std::fs::read_to_string(&args[2]).expect("Unable to read input file");
            let parsed_schema = parse_schema(&supergraph_sdl);
            let supergraph = SupergraphState::new(&parsed_schema);
            let graph =
                Graph::graph_from_supergraph_state(&supergraph).expect("failed to create graph");
            println!("{}", graph);
        }
        "paths" => {
            let (graph, best_paths_per_leaf, _operation, _supergraph_state) =
                process_paths(&args[2], &args[3]);

            for (index, best_path) in best_paths_per_leaf
                .root_field_groups
                .iter()
                .flatten()
                .enumerate()
            {
                println!(
                    "Path at index {} has total of {} best paths:",
                    index,
                    best_path.len(),
                );

                for path in best_path {
                    println!("    {}", path.pretty_print(&graph));
                }
            }
        }
        "tree" => {
            let (graph, query_tree, _supergraph_state) = process_merged_tree(&args[2], &args[3]);

            println!(
                "{}",
                query_tree
                    .pretty_print(&graph)
                    .expect("failed to print merged tree")
            )
        }
        "fetch_graph" => {
            let fetch_graph = process_fetch_graph(&args[2], &args[3]);
            println!("{}", fetch_graph);
        }
        "plan" => {
            let plan = process_plan(&args[2], &args[3]);
            if args.contains(&"--json".into()) {
                println!("{}", serde_json::to_string_pretty(&plan).unwrap());
            } else {
                println!("{}", plan);
            }
        }
        "normalize" => {
            let supergraph_sdl =
                std::fs::read_to_string(&args[2]).expect("Unable to read input file");
            let parsed_schema = parse_schema(&supergraph_sdl);
            let supergraph = SupergraphState::new(&parsed_schema);
            let document_text =
                std::fs::read_to_string(&args[3]).expect("Unable to read input file");
            let parsed_document = parse_operation(&document_text);
            let document = normalize_operation(&supergraph, &parsed_document, None).unwrap();
            let operation = document.executable_operation();

            println!("{}", operation);
        }
        _ => {
            eprintln!("Unknown command. Available commands: consumer_graph, graph, paths, tree, fetch_graph, plan");
            process::exit(1);
        }
    };
}

fn process_consumer_schema(path: &str) {
    let supergraph_sdl = std::fs::read_to_string(path).expect("Unable to read input file");
    let parsed_schema = parse_schema(&supergraph_sdl);
    let consumer_schema = ConsumerSchema::new_from_supergraph(&parsed_schema);

    println!("{}", consumer_schema.document);
}

fn process_fetch_graph(supergraph_path: &str, operation_path: &str) -> FetchGraph {
    let (graph, query_tree, supergraph_state) =
        process_merged_tree(supergraph_path, operation_path);

    build_fetch_graph_from_query_tree(&graph, &supergraph_state, query_tree)
        .expect("failed to build fetch graph")
}

fn process_plan(supergraph_path: &str, operation_path: &str) -> QueryPlan {
    let supergraph_sdl =
        std::fs::read_to_string(supergraph_path).expect("Unable to read input file");
    let parsed_schema = parse_schema(&supergraph_sdl);
    let supergraph = SupergraphState::new(&parsed_schema);
    let graph = Graph::graph_from_supergraph_state(&supergraph).expect("failed to create graph");
    let operation = get_operation(operation_path, &supergraph);
    let best_paths_per_leaf = walk_operation(&graph, &operation).unwrap();
    let query_tree = find_best_combination(&graph, best_paths_per_leaf).unwrap();
    let fetch_graph = build_fetch_graph_from_query_tree(&graph, &supergraph, query_tree)
        .expect("failed to build fetch graph");

    build_query_plan_from_fetch_graph(fetch_graph, &supergraph).expect("failed to build query plan")
}

fn process_merged_tree(
    supergraph_path: &str,
    operation_path: &str,
) -> (Graph, QueryTree, SupergraphState) {
    let (graph, best_paths_per_leaf, _operation, supergraph_state) =
        process_paths(supergraph_path, operation_path);
    let query_tree = find_best_combination(&graph, best_paths_per_leaf).unwrap();

    (graph, query_tree, supergraph_state)
}

fn get_operation(operation_path: &str, supergraph: &SupergraphState) -> OperationDefinition {
    let document_text = std::fs::read_to_string(operation_path).expect("Unable to read input file");
    let parsed_document = parse_operation(&document_text);
    let document = normalize_operation(supergraph, &parsed_document, None).unwrap();
    let operation = document.executable_operation();

    operation.clone()
}

fn process_paths(
    supergraph_path: &str,
    operation_path: &str,
) -> (
    Graph,
    ResolvedOperation,
    OperationDefinition,
    SupergraphState,
) {
    let supergraph_sdl =
        std::fs::read_to_string(supergraph_path).expect("Unable to read input file");
    let parsed_schema = parse_schema(&supergraph_sdl);
    let supergraph = SupergraphState::new(&parsed_schema);
    let graph = Graph::graph_from_supergraph_state(&supergraph).expect("failed to create graph");
    let operation = get_operation(operation_path, &supergraph);
    let best_paths_per_leaf = walk_operation(&graph, &operation).unwrap();

    (graph, best_paths_per_leaf, operation, supergraph)
}
