use std::path::Path;

use hive_router_config::log::{
    service::{FileExporterConfig, FileRolling},
    shared::LogFormat,
};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{fmt::time::UtcTime, Layer};

use crate::logging::utils::{create_targets_filter, DynLayer};

pub fn build_file_layer<S>(config: &FileExporterConfig) -> (DynLayer<S>, WorkerGuard)
where
    S: tracing::Subscriber
        + for<'span> tracing_subscriber::registry::LookupSpan<'span>
        + Send
        + Sync,
{
    let path = Path::new(&config.path);
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .expect("oops");
    let directory = path.parent().and_then(|p| p.to_str()).expect("oops");

    let (file_writer, file_guard) = tracing_appender::non_blocking(match &config.rolling {
        None => tracing_appender::rolling::never(directory, file_name),
        Some(rolling) => match rolling {
            FileRolling::Daily => tracing_appender::rolling::daily(directory, file_name),
            FileRolling::Hourly => tracing_appender::rolling::hourly(directory, file_name),
            FileRolling::Minutely => tracing_appender::rolling::minutely(directory, file_name),
        },
    });

    let filter = create_targets_filter(&config.level, config.log_internals);
    let timer = UtcTime::rfc_3339();
    let file_layer = tracing_subscriber::fmt::layer().with_writer(file_writer);

    let layer = match config.format {
        LogFormat::Json => file_layer
            .json()
            .with_timer(timer)
            .with_thread_ids(false)
            .with_target(false)
            .flatten_event(true)
            .with_filter(filter)
            .boxed(),
        LogFormat::Text => file_layer
            .compact()
            .with_thread_ids(false)
            .with_target(false)
            .with_timer(timer)
            .with_filter(filter)
            .boxed(),
    };

    (layer, file_guard)
}
