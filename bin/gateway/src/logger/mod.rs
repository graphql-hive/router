use std::str::FromStr;

use gateway_config::log::{LogFormat, LoggingConfig};
use tracing_subscriber::{
    fmt::{self, format::FmtSpan, time::UtcTime},
    layer::SubscriberExt,
    util::SubscriberInitExt,
    EnvFilter, Layer, Registry,
};

pub fn configure_logging(config: &LoggingConfig) {
    let timer = UtcTime::rfc_3339();
    let filter = EnvFilter::from_str(config.env_filter_str())
        .unwrap_or_else(|e| panic!("failed to initialize env-filter logger: {}", e));

    let layer = match config.format {
        LogFormat::PrettyTree => tracing_tree::HierarchicalLayer::new(2)
            .with_bracketed_fields(true)
            .with_deferred_spans(false)
            .with_wraparound(25)
            .with_indent_lines(true)
            .with_timer(tracing_tree::time::Uptime::default())
            .with_thread_names(false)
            .with_thread_ids(false)
            .with_targets(false)
            .boxed(),
        LogFormat::Json => fmt::Layer::<Registry>::default()
            .json()
            .with_timer(timer)
            .with_span_events(FmtSpan::CLOSE)
            .boxed(),
        LogFormat::PrettyCompact => fmt::Layer::<Registry>::default()
            .compact()
            .with_timer(timer)
            .with_span_events(FmtSpan::CLOSE)
            .boxed(),
    };

    let registry = tracing_subscriber::registry();
    let registry = registry.with(layer.boxed()).with(filter.boxed());
    registry.init();
}
