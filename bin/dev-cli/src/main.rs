use std::env;
use std::process;

use hive_router_plan_executor::introspection::schema::SchemaWithMetadata;
use hive_router_plan_executor::projection::plan::FieldProjectionPlan;
use hive_router_query_planner::ast::normalization::normalize_operation;
use hive_router_query_planner::ast::operation::OperationDefinition;
use hive_router_query_planner::consumer_schema::ConsumerSchema;
use hive_router_query_planner::graph::Graph;
use hive_router_query_planner::graph::PlannerOverrideContext;
use hive_router_query_planner::planner::best::find_best_combination;
use hive_router_query_planner::planner::fetch::fetch_graph::build_fetch_graph_from_query_tree;
use hive_router_query_planner::planner::fetch::fetch_graph::FetchGraph;
use hive_router_query_planner::planner::fetch::state::MultiTypeFetchStep;
use hive_router_query_planner::planner::plan_nodes::QueryPlan;
use hive_router_query_planner::planner::query_plan::build_query_plan_from_fetch_graph;
use hive_router_query_planner::planner::tree::query_tree::QueryTree;
use hive_router_query_planner::planner::walker::walk_operation;
use hive_router_query_planner::planner::QueryPlannerOptions;
use hive_router_query_planner::state::supergraph_state::OperationKind;
use hive_router_query_planner::state::supergraph_state::SupergraphState;
use hive_router_query_planner::utils::cancellation::CancellationToken;
use hive_router_query_planner::utils::parsing::parse_operation;
use hive_router_query_planner::utils::parsing::parse_schema;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

fn measure_time<F: FnOnce() -> R, R>(name: &str, f: F) -> R {
    let start = std::time::Instant::now();
    let result = f();
    let duration = start.elapsed();
    println!("{}: {:?}", name, duration);
    result
}

fn main() {
    let tree_layer = tracing_tree::HierarchicalLayer::new(2)
        .with_writer(std::io::stdout)
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
            let (graph, operation, supergraph_state) = load_graph_operation(&args[2], &args[3]);
            let override_context = PlannerOverrideContext::default();
            let cancellation_token = CancellationToken::new();
            let best_paths_per_leaf = walk_operation(
                &graph,
                &supergraph_state,
                &override_context,
                &operation,
                &cancellation_token,
            )
            .unwrap();

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
        "projection" => {
            let supergraph_sdl =
                std::fs::read_to_string(&args[2]).expect("Unable to read input file");
            let parsed_schema = parse_schema(&supergraph_sdl);
            let supergraph = SupergraphState::new(&parsed_schema);
            let document_text =
                std::fs::read_to_string(&args[3]).expect("Unable to read input file");
            let parsed_document = parse_operation(&document_text);
            let document = normalize_operation(&supergraph, &parsed_document, None).unwrap();
            let operation = document.executable_operation();
            let consumer_schema = ConsumerSchema::new_from_supergraph(&parsed_schema);
            let schema_metadata = consumer_schema.schema_metadata();

            let (_, projection_plan) =
                FieldProjectionPlan::from_operation(operation, &schema_metadata);

            for plan in &projection_plan {
                println!("{}", plan);
            }
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

fn process_fetch_graph<'a>(
    supergraph_path: &'a str,
    operation_path: &'a str,
) -> FetchGraph<'a, MultiTypeFetchStep> {
    let (graph, query_tree, supergraph_state, operation_kind) =
        process_merged_tree(supergraph_path, operation_path);

    let override_context = PlannerOverrideContext::default();
    let cancellation_token = CancellationToken::new();
    build_fetch_graph_from_query_tree(
        &graph,
        &supergraph_state,
        &override_context,
        query_tree,
        operation_kind,
        &QueryPlannerOptions::default(),
        &cancellation_token,
    )
    .expect("failed to build fetch graph")
}

fn process_plan(supergraph_path: &str, operation_path: &str) -> QueryPlan {
    let (graph, operation, supergraph) = load_graph_operation(supergraph_path, operation_path);
    let override_context = PlannerOverrideContext::default();
    let cancellation_token = CancellationToken::new();

    let now = std::time::Instant::now();
    let best_paths_per_leaf = measure_time("walk_operation", || {
        walk_operation(
            &graph,
            &supergraph,
            &override_context,
            &operation,
            &cancellation_token,
        )
        .unwrap()
    });
    let query_tree = measure_time("find_best_combination", || {
        find_best_combination(&graph, best_paths_per_leaf, &cancellation_token).unwrap()
    });
    let fetch_graph = measure_time("build_fetch_graph_from_query_tree", || {
        build_fetch_graph_from_query_tree(
            &graph,
            &supergraph,
            &override_context,
            query_tree,
            operation
                .operation_kind
                .clone()
                .unwrap_or(OperationKind::Query),
            &QueryPlannerOptions::default(),
            &cancellation_token,
        )
        .expect("failed to build fetch graph")
    });
    let qp = measure_time("build_query_plan_from_fetch_graph", || {
        build_query_plan_from_fetch_graph(fetch_graph, &supergraph, &cancellation_token)
            .expect("failed to build query plan")
    });
    println!("total: {:?}", now.elapsed());

    qp
}

fn process_merged_tree<'a>(
    supergraph_path: &'a str,
    operation_path: &'a str,
) -> (Graph<'static>, QueryTree, &'static SupergraphState, OperationKind) {
    let (graph, operation, supergraph_state) =
        load_graph_operation(supergraph_path, operation_path);
    let override_context = PlannerOverrideContext::default();
    let cancellation_token = CancellationToken::new();
    let best_paths_per_leaf = walk_operation(
        &graph,
        supergraph_state,
        &override_context,
        &operation,
        &cancellation_token,
    )
    .unwrap();
    let query_tree =
        find_best_combination(&graph, best_paths_per_leaf, &cancellation_token).unwrap();

    (
        graph,
        query_tree,
        supergraph_state,
        operation
            .operation_kind
            .clone()
            .unwrap_or(OperationKind::Query),
    )
}

fn get_operation(operation_path: &str, supergraph: &SupergraphState) -> OperationDefinition {
    let document_text = std::fs::read_to_string(operation_path).expect("Unable to read input file");
    let parsed_document = parse_operation(&document_text);
    let document = normalize_operation(supergraph, &parsed_document, None).unwrap();
    let operation = document.executable_operation();

    operation.clone()
}

fn load_graph_operation<'a>(
    supergraph_path: &'a str,
    operation_path: &'a str,
) -> (Graph<'static>, OperationDefinition, &'static SupergraphState) {
    let supergraph_sdl =
        std::fs::read_to_string(supergraph_path).expect("Unable to read input file");
    let parsed_schema = measure_time("parse_schema", || parse_schema(&supergraph_sdl));
    let supergraph = measure_time("SupergraphState::new", || {
        Box::leak(Box::new(SupergraphState::new(&parsed_schema)))
    });
    let graph = measure_time("Graph::graph_from_supergraph_state", || {
        Graph::graph_from_supergraph_state(supergraph)
    })
    .expect("failed to create graph");
    let operation = measure_time("get_operation", || {
        get_operation(operation_path, supergraph)
    });

    (graph, operation, supergraph)
}
