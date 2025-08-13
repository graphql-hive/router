use tracing_subscriber::{
    fmt::{self, format::FmtSpan, time::UtcTime},
    layer::SubscriberExt,
    util::SubscriberInitExt,
    EnvFilter, Layer, Registry,
};

#[allow(dead_code)]
pub enum LoggingFormat {
    PrettyTree,
    PrettyCompact,
    Json,
}

pub fn configure_logging(format: LoggingFormat) {
    let timer = UtcTime::rfc_3339();
    let filter = EnvFilter::from_default_env();

    let layer = match format {
        LoggingFormat::PrettyTree => tracing_tree::HierarchicalLayer::new(2)
            .with_bracketed_fields(true)
            .with_deferred_spans(false)
            .with_wraparound(25)
            .with_indent_lines(true)
            .with_timer(tracing_tree::time::Uptime::default())
            .with_thread_names(false)
            .with_thread_ids(false)
            .with_targets(false)
            .boxed(),
        LoggingFormat::Json => fmt::Layer::<Registry>::default()
            .json()
            .with_timer(timer)
            .with_span_events(FmtSpan::CLOSE)
            .boxed(),
        LoggingFormat::PrettyCompact => fmt::Layer::<Registry>::default()
            .compact()
            .with_timer(timer)
            .with_span_events(FmtSpan::CLOSE)
            .boxed(),
    };

    let registry = tracing_subscriber::registry();
    let registry = registry.with(layer.boxed()).with(filter.boxed());
    registry.init();
}
