use std::env;
use std::process;

use graphql_differential::{GeneratorConfig, QueryGenerator};
use graphql_tools::parser::schema::{Definition, TypeDefinition};
use graphql_tools::validation::{rules::default_rules_validation_plan, validate::validate};
use hive_router::executor::introspection::schema::SchemaWithMetadata;
use hive_router::executor::projection::plan::ProjectionPlan;
use hive_router::query_planner::ast::normalization::normalize_operation;
use hive_router::query_planner::ast::operation::OperationDefinition;
use hive_router::query_planner::consumer_schema::ConsumerSchema;
use hive_router::query_planner::graph::Graph;
use hive_router::query_planner::graph::PlannerOverrideContext;
use hive_router::query_planner::planner::best::find_best_combination;
use hive_router::query_planner::planner::fetch::fetch_graph::build_fetch_graph_from_query_tree;
use hive_router::query_planner::planner::fetch::fetch_graph::FetchGraph;
use hive_router::query_planner::planner::fetch::state::MultiTypeFetchStep;
use hive_router::query_planner::planner::plan_nodes::planning::QueryPlan;
use hive_router::query_planner::planner::query_plan::build_query_plan_from_fetch_graph;
use hive_router::query_planner::planner::tree::query_tree::QueryTree;
use hive_router::query_planner::planner::walker::walk_operation;
use hive_router::query_planner::planner::QueryPlannerOptions;
use hive_router::query_planner::state::supergraph_state::SupergraphState;
use hive_router::query_planner::utils::cancellation::CancellationToken;
use hive_router::query_planner::utils::parsing::parse_operation;
use hive_router::query_planner::utils::parsing::parse_schema;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

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
                supergraph_state,
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

            let (_, projection) = ProjectionPlan::from_operation(operation, &schema_metadata);
            println!("{projection}");
        }
        // `generate <supergraph> <count> [first seed]`, `count` operations of the differential
        // generator the router accepts, one JSON object a line:
        // `{ seed, document, operationName, variables }`
        "generate" => {
            let supergraph_sdl =
                std::fs::read_to_string(&args[2]).expect("Unable to read input file");
            let count: u64 = args.get(3).and_then(|n| n.parse().ok()).unwrap_or(10);
            let first: u64 = args.get(4).and_then(|n| n.parse().ok()).unwrap_or(0);
            generate_operations(&supergraph_sdl, count, first);
        }
        _ => {
            eprintln!("Unknown command. Available commands: consumer_schema, graph, paths, fetch_graph, plan, generate");
            process::exit(1);
        }
    };
}

fn generate_operations(supergraph_sdl: &str, count: u64, first: u64) {
    let supergraph = parse_schema(supergraph_sdl);
    let schema = ConsumerSchema::new_from_supergraph(&supergraph);
    // without introspection, the generator would spend most operations on `__schema`
    let mut api = (*schema.document).clone();
    api.definitions.retain_mut(|definition| match definition {
        Definition::TypeDefinition(TypeDefinition::Object(object)) => {
            object.fields.retain(|field| !field.name.starts_with("__"));
            !object.name.starts_with("__")
        }
        Definition::TypeDefinition(other) => !other.name().starts_with("__"),
        _ => true,
    });
    let config = GeneratorConfig {
        max_depth: 6,
        max_width: 6,
        ..GeneratorConfig::default()
    };
    let rules = default_rules_validation_plan();
    let debug = std::env::var("GENERATE_DEBUG").is_ok();
    // generated operations are often invalid, so try more seeds until there are `count` valid
    // ones, up to this many seeds per operation
    const SEEDS_PER_OPERATION: u64 = 50;
    let mut valid = 0;
    for seed in first..first + count * SEEDS_PER_OPERATION {
        if valid == count {
            break;
        }
        let case = QueryGenerator::new(&api, seed, config.clone()).generate();
        // the generator doesn't always make a valid operation, and the router refuses those
        let errors = validate(&schema.document, &parse_operation(&case.document), &rules);
        if !errors.is_empty() {
            if debug {
                eprintln!("{seed}: {:?}\n{}", errors.first(), case.document);
            }
            continue;
        }
        valid += 1;
        let variables: serde_json::Value =
            serde_json::from_str(&case.variables_json).expect("generated variables are JSON");
        println!(
            "{}",
            serde_json::json!({
                "seed": seed,
                "document": case.document,
                "operationName": case.operation_name,
                "variables": variables,
            })
        );
    }
}

fn process_consumer_schema(path: &str) {
    let supergraph_sdl = std::fs::read_to_string(path).expect("Unable to read input file");
    let parsed_schema = parse_schema(&supergraph_sdl);
    let consumer_schema = ConsumerSchema::new_from_supergraph(&parsed_schema);

    println!("{}", consumer_schema.document);
}

fn process_fetch_graph(
    supergraph_path: &str,
    operation_path: &str,
) -> FetchGraph<MultiTypeFetchStep> {
    let (graph, query_tree, supergraph_state, operation) =
        process_merged_tree(supergraph_path, operation_path);

    let override_context = PlannerOverrideContext::default();
    let cancellation_token = CancellationToken::new();
    build_fetch_graph_from_query_tree(
        &graph,
        supergraph_state,
        &override_context,
        query_tree,
        &operation,
        &QueryPlannerOptions::default(),
        &cancellation_token,
    )
    .expect("failed to build fetch graph")
}

fn process_plan(supergraph_path: &str, operation_path: &str) -> QueryPlan {
    let (graph, operation, supergraph) = load_graph_operation(supergraph_path, operation_path);
    let override_context = PlannerOverrideContext::default();
    let cancellation_token = CancellationToken::new();

    let best_paths_per_leaf = walk_operation(
        &graph,
        supergraph,
        &override_context,
        &operation,
        &cancellation_token,
    )
    .unwrap();
    let query_tree =
        find_best_combination(&graph, best_paths_per_leaf, &cancellation_token).unwrap();
    let fetch_graph = build_fetch_graph_from_query_tree(
        &graph,
        supergraph,
        &override_context,
        query_tree,
        &operation,
        &QueryPlannerOptions::default(),
        &cancellation_token,
    )
    .expect("failed to build fetch graph");

    build_query_plan_from_fetch_graph(
        fetch_graph,
        supergraph,
        &QueryPlannerOptions::default(),
        &cancellation_token,
    )
    .expect("failed to build query plan")
}

fn process_merged_tree(
    supergraph_path: &str,
    operation_path: &str,
) -> (
    Graph<'static>,
    QueryTree,
    &'static SupergraphState,
    OperationDefinition,
) {
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

    (graph, query_tree, supergraph_state, operation)
}

fn get_operation(operation_path: &str, supergraph: &SupergraphState) -> OperationDefinition {
    let document_text = std::fs::read_to_string(operation_path).expect("Unable to read input file");
    let parsed_document = parse_operation(&document_text);
    let document = normalize_operation(supergraph, &parsed_document, None).unwrap();
    let operation = document.executable_operation();

    operation.clone()
}

fn load_graph_operation(
    supergraph_path: &str,
    operation_path: &str,
) -> (
    Graph<'static>,
    OperationDefinition,
    &'static SupergraphState,
) {
    let supergraph_sdl =
        std::fs::read_to_string(supergraph_path).expect("Unable to read input file");
    let parsed_schema = parse_schema(&supergraph_sdl);
    let supergraph = Box::leak(Box::new(SupergraphState::new(&parsed_schema)));
    let graph = Graph::graph_from_supergraph_state(supergraph).expect("failed to create graph");
    let operation = get_operation(operation_path, supergraph);

    (graph, operation, supergraph)
}
