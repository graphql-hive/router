use std::env;
use std::error::Error;
use std::path::PathBuf;
use std::sync::Once;

use lazy_static::lazy_static;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

use graphql_tools::parser::query as query_ast;

use crate::query_planner::ast::normalization::normalize_operation;
use crate::query_planner::graph::edge::PlannerOverrideContext;
use crate::query_planner::planner::plan_nodes::planning::QueryPlan;
use crate::query_planner::planner::{Planner, QueryPlannerOptions};
use crate::query_planner::utils::cancellation::CancellationToken;
use crate::query_planner::utils::parsing::parse_schema;

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

pub fn build_query_plan(
    fixture_path: &str,
    query: query_ast::Document<'static, String>,
    override_context: PlannerOverrideContext,
    options: QueryPlannerOptions,
) -> Result<QueryPlan, Box<dyn Error>> {
    let schema = parse_schema(&read_supergraph(fixture_path));
    let planner = Planner::new_from_supergraph(&schema, options)?;
    let document = normalize_operation(&planner.supergraph, &query, None)?;
    let plan = planner.plan_from_normalized_operation(
        document.executable_operation(),
        override_context,
        &CancellationToken::new(),
    )?;

    Ok(plan)
}

pub fn build_query_plan_with_defaults(
    fixture_path: &str,
    query: query_ast::Document<'static, String>,
) -> Result<QueryPlan, Box<dyn Error>> {
    build_query_plan(
        fixture_path,
        query,
        PlannerOverrideContext::default(),
        Default::default(),
    )
}
