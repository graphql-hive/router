use std::env;
use std::path::PathBuf;
use std::sync::Once;

use lazy_static::lazy_static;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

use crate::consumer_schema::ConsumerSchema;
use crate::graph::Graph;
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

pub fn read_supergraph(fixture_path: &str) -> (Graph, ConsumerSchema) {
    let supergraph_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(fixture_path);
    let supergraph_sdl =
        std::fs::read_to_string(supergraph_path).expect("Unable to read input file");
    let schema = parse_schema(&supergraph_sdl);
    let supergraph_state = SupergraphState::new(&schema);

    (
        Graph::graph_from_supergraph_state(&supergraph_state).expect("failed to create graph"),
        ConsumerSchema::new_from_supergraph(&schema),
    )
}
