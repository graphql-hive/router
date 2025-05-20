use std::env;
use std::path::PathBuf;
use std::sync::Once;

use lazy_static::lazy_static;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use crate::graph::Graph;
use crate::parse_schema;
use crate::planner::tree::query_tree::QueryTree;
use crate::planner::walker::path::OperationPath;
use crate::state::supergraph_state::SupergraphState;

pub fn paths_to_trees(graph: &Graph, paths: &[Vec<OperationPath>]) -> Vec<QueryTree> {
    paths
        .iter()
        .map(|paths| {
            QueryTree::from_path(graph, &paths[0]).expect("expected tree to be built but it failed")
        })
        .collect::<Vec<_>>()
}

fn init_test_logger_internal() {
    let tree_layer = tracing_tree::HierarchicalLayer::new(2)
        .with_bracketed_fields(true)
        .with_deferred_spans(true)
        .with_wraparound(25)
        .with_indent_lines(true)
        .with_timer(tracing_tree::time::Uptime::default())
        .with_thread_names(false)
        .with_thread_ids(false)
        .with_targets(false);

    tracing_subscriber::registry().with(tree_layer).init();
}

lazy_static! {
    static ref TRACING_INIT: Once = Once::new();
}

pub fn init_logger() {
    TRACING_INIT.call_once(|| {
        let logger_enabled = env::var("DEBUG").is_ok();

        if logger_enabled {
            init_test_logger_internal();
        } else {
            println!("Logger is disabled, to print QP logs, please set DEBUG=1")
        }
    });
}

pub fn read_supergraph(fixture_path: &str) -> Graph {
    let supergraph_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(fixture_path);
    let supergraph_sdl =
        std::fs::read_to_string(supergraph_path).expect("Unable to read input file");
    let schema = parse_schema(&supergraph_sdl);
    let supergraph_state = SupergraphState::new(&schema);

    Graph::graph_from_supergraph_state(&supergraph_state).expect("failed to create graph")
}
