use std::io::IsTerminal;

use hive_router_config::log::{service::StdoutExporterConfig, shared::LogFormat};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{fmt::time::UtcTime, Layer};

use crate::logging::utils::{create_env_filter, DynLayer};

pub fn build_stdout_layer<S>(config: &StdoutExporterConfig) -> (DynLayer<S>, WorkerGuard)
where
    S: tracing::Subscriber
        + for<'span> tracing_subscriber::registry::LookupSpan<'span>
        + Send
        + Sync,
{
    let stdout_stream = std::io::stdout();
    let is_terminal = stdout_stream.is_terminal();
    let (stdout_writer, stdout_guard) = tracing_appender::non_blocking(stdout_stream);
    let filter = create_env_filter(&config.level, config.log_internals);
    let stdout_layer = tracing_subscriber::fmt::layer().with_writer(stdout_writer);
    let timer = UtcTime::rfc_3339();

    let layer = match config.format {
        LogFormat::Json => stdout_layer
            .json()
            .with_timer(timer)
            .with_thread_ids(false)
            .with_target(false)
            .with_ansi(is_terminal)
            .flatten_event(true)
            .with_filter(filter)
            .boxed(),
        LogFormat::Text => stdout_layer
            .compact()
            .with_thread_ids(false)
            .with_timer(timer)
            .with_target(false)
            .with_ansi(is_terminal)
            .with_filter(filter)
            .boxed(),
    };

    (layer, stdout_guard)
}
