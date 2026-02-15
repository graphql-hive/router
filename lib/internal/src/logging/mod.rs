pub mod file;
pub mod logger_span;
pub mod request_id;
pub mod stdout;
pub mod utils;

use crate::logging::{file::build_file_layer, stdout::build_stdout_layer, utils::DynLayer};
use hive_router_config::log::{service::ServiceLogExporter, LoggingConfig};
use tracing_appender::non_blocking::WorkerGuard;

pub fn logging_layers_from_logger_config<S>(
    config: &LoggingConfig,
) -> (Vec<DynLayer<S>>, Vec<WorkerGuard>)
where
    S: tracing::Subscriber
        + for<'span> tracing_subscriber::registry::LookupSpan<'span>
        + Send
        + Sync,
{
    let mut layers = vec![];
    let mut guards = vec![];

    for service_layer_config in config.service.as_list().exporters {
        let out = match service_layer_config {
            ServiceLogExporter::Stdout(config) => build_stdout_layer(&config),
            ServiceLogExporter::File(config) => build_file_layer(&config),
        };

        layers.push(out.0);
        guards.push(out.1);
    }

    (layers, guards)
}
