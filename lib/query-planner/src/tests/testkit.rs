use std::env;
use std::error::Error;
use std::path::PathBuf;
use std::sync::Once;

use lazy_static::lazy_static;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

use graphql_parser::query as query_ast;

use crate::ast::normalization::normalize_operation;
use crate::graph::edge::PlannerOverrideContext;
use crate::graph::Graph;
use crate::planner::add_variables_to_fetch_steps;
use crate::planner::best::find_best_combination;
use crate::planner::fetch::fetch_graph::build_fetch_graph_from_query_tree;
use crate::planner::plan_nodes::QueryPlan;
use crate::planner::query_plan::build_query_plan_from_fetch_graph;
use crate::planner::walker::walk_operation;
use crate::state::supergraph_state::SupergraphState;
use crate::utils::parsing::parse_schema;

fn init_test_logger_internal() {
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
}

lazy_static! {
    static ref TRACING_INIT: Once = Once::new();
}

pub fn init_logger() {
    TRACING_INIT.call_once(|| {
        init_test_logger_internal();
    });
}

pub fn read_supergraph(fixture_path: &str) -> String {
    let supergraph_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(fixture_path);

    std::fs::read_to_string(supergraph_path).expect("Unable to read input file")
}

pub fn build_query_plan_with_context(
    fixture_path: &str,
    query: query_ast::Document<'static, String>,
    override_context: PlannerOverrideContext,
) -> Result<QueryPlan, Box<dyn Error>> {
    let schema = parse_schema(&read_supergraph(fixture_path));
    let supergraph_state = SupergraphState::new(&schema);
    let graph =
        Graph::graph_from_supergraph_state(&supergraph_state).expect("failed to create graph");
    let document = normalize_operation(&supergraph_state, &query, None).unwrap();
    let operation = document.executable_operation();
    let best_paths_per_leaf = walk_operation(&graph, &override_context, operation)?;
    let query_tree = find_best_combination(&graph, best_paths_per_leaf).unwrap();
    let mut fetch_graph = build_fetch_graph_from_query_tree(
        &graph,
        &supergraph_state,
        &override_context,
        query_tree,
    )?;
    add_variables_to_fetch_steps(&mut fetch_graph, &operation.variable_definitions)?;

    let plan = build_query_plan_from_fetch_graph(fetch_graph, &supergraph_state)?;

    Ok(plan)
}

pub fn build_query_plan(
    fixture_path: &str,
    query: query_ast::Document<'static, String>,
) -> Result<QueryPlan, Box<dyn Error>> {
    build_query_plan_with_context(fixture_path, query, PlannerOverrideContext::default())
}
