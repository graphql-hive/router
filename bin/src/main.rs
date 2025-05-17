use std::env;
use std::process;

use query_planner::consumer_schema::ConsumerSchema;
use query_planner::graph::Graph;
use query_planner::parse_operation;
use query_planner::parse_schema;
use query_planner::planner::fetch::fetch_graph::build_fetch_graph_from_query_tree;
use query_planner::planner::tree::query_tree::QueryTree;
use query_planner::planner::walker::walk_operation;
use query_planner::state::supergraph_state::SupergraphState;
use query_planner::utils::operation_utils::get_operation_to_execute;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

fn main() {
    let logger_enabled = env::var("DEBUG").is_ok();

    if logger_enabled {
        let tree_layer = tracing_tree::HierarchicalLayer::new(2)
            .with_bracketed_fields(true)
            .with_deferred_spans(true)
            .with_wraparound(15)
            .with_indent_lines(true)
            .with_timer(tracing_tree::time::Uptime::default())
            .with_thread_names(false)
            .with_thread_ids(false)
            .with_targets(false);

        tracing_subscriber::registry().with(tree_layer).init();
    }

    let args: Vec<String> = env::args().collect();

    if args.len() < 3 {
        eprintln!("Usage: query-planner <command> <supergraph_path> [...]");
        process::exit(1);
    }

    match args[1].as_str() {
        "consumer_schema" => process_consumer_schema(&args[2]),
        "graph" => process_graph(&args[2]),
        "paths" => process_paths(&args[2], &args[3]),
        "trees" => process_trees(&args[2], &args[3]),
        "merged_tree" => process_merged_tree(&args[2], &args[3]),
        "fetch_graph" => process_fetch_graph(&args[2], &args[3]),
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
    let consumer_schema = ConsumerSchema::new_from_supergraph(&parsed_schema);

    println!("{}", consumer_schema.document);
}

fn process_graph(path: &str) {
    let supergraph_sdl = std::fs::read_to_string(path).expect("Unable to read input file");
    let parsed_schema = parse_schema(&supergraph_sdl);
    let supergraph_state = SupergraphState::new(&parsed_schema);
    let graph =
        Graph::graph_from_supergraph_state(&supergraph_state).expect("failed to create graph");

    println!("{}", graph);
}

fn process_trees(supergraph_path: &str, operation_path: &str) {
    let supergraph_sdl =
        std::fs::read_to_string(supergraph_path).expect("Unable to read input file");
    let operation_text =
        std::fs::read_to_string(operation_path).expect("Unable to read input file");
    let parsed_schema = parse_schema(&supergraph_sdl);
    let operation = parse_operation(&operation_text);
    let supergraph_state = SupergraphState::new(&parsed_schema);
    let graph =
        Graph::graph_from_supergraph_state(&supergraph_state).expect("failed to create graph");
    let operation = get_operation_to_execute(&operation).expect("failed to locate operation");
    let best_paths_per_leaf = walk_operation(&graph, operation).unwrap();

    for (index, best_path) in best_paths_per_leaf.iter().enumerate() {
        println!(
            "Path at index {} has total of {} paths, trees:",
            index,
            best_path.len(),
        );

        for (tree_index, path) in best_path.iter().enumerate() {
            println!("== #{}: {} ==", tree_index, path.pretty_print(&graph));
            println!(
                "{}",
                QueryTree::from_path(&graph, &path)
                    .expect("expected tree to be built but it failed")
                    .pretty_print(&graph)
                    .expect("failed to print tree")
            );
            println!("");
        }
    }
}

fn process_fetch_graph(supergraph_path: &str, operation_path: &str) {
    let supergraph_sdl =
        std::fs::read_to_string(supergraph_path).expect("Unable to read input file");
    let operation_text =
        std::fs::read_to_string(operation_path).expect("Unable to read input file");
    let parsed_schema = parse_schema(&supergraph_sdl);
    let operation = parse_operation(&operation_text);
    let supergraph_state = SupergraphState::new(&parsed_schema);
    let graph =
        Graph::graph_from_supergraph_state(&supergraph_state).expect("failed to create graph");
    let operation = get_operation_to_execute(&operation).expect("failed to locate operation");
    let best_paths_per_leaf = walk_operation(&graph, operation).unwrap();
    let qtps = best_paths_per_leaf
        .iter()
        .map(|paths| {
            QueryTree::from_path(&graph, &paths[0])
                .expect("expected tree to be built but it failed")
        })
        .collect::<Vec<_>>();
    let query_tree = QueryTree::merge_trees(qtps);
    let fetch_graph =
        build_fetch_graph_from_query_tree(&graph, query_tree).expect("failed to build fetch graph");

    println!("{}", fetch_graph);
}

fn process_merged_tree(supergraph_path: &str, operation_path: &str) {
    let supergraph_sdl =
        std::fs::read_to_string(supergraph_path).expect("Unable to read input file");
    let operation_text =
        std::fs::read_to_string(operation_path).expect("Unable to read input file");
    let parsed_schema = parse_schema(&supergraph_sdl);
    let operation = parse_operation(&operation_text);
    let supergraph_state = SupergraphState::new(&parsed_schema);
    let graph =
        Graph::graph_from_supergraph_state(&supergraph_state).expect("failed to create graph");
    let operation = get_operation_to_execute(&operation).expect("failed to locate operation");
    let best_paths_per_leaf = walk_operation(&graph, operation).unwrap();
    let qtps = best_paths_per_leaf
        .iter()
        .map(|paths| {
            QueryTree::from_path(&graph, &paths[0])
                .expect("expected tree to be built but it failed")
        })
        .collect::<Vec<_>>();
    let query_tree = QueryTree::merge_trees(qtps);

    println!(
        "{}",
        query_tree
            .pretty_print(&graph)
            .expect("failed to print merged tree")
    )
}

fn process_paths(supergraph_path: &str, operation_path: &str) {
    let supergraph_sdl =
        std::fs::read_to_string(supergraph_path).expect("Unable to read input file");
    let operation_text =
        std::fs::read_to_string(operation_path).expect("Unable to read input file");
    let parsed_schema = parse_schema(&supergraph_sdl);
    let operation = parse_operation(&operation_text);
    let supergraph_state = SupergraphState::new(&parsed_schema);
    let graph =
        Graph::graph_from_supergraph_state(&supergraph_state).expect("failed to create graph");
    let operation = get_operation_to_execute(&operation).expect("failed to locate operation");
    let best_paths_per_leaf = walk_operation(&graph, operation).unwrap();

    for (index, best_path) in best_paths_per_leaf.iter().enumerate() {
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
