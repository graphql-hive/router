use std::time::Duration;

use hive_router_config::telemetry::TelemetryConfig;
use opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge;
use opentelemetry_sdk::logs::{BatchConfigBuilder, BatchLogProcessor};
use opentelemetry_sdk::logs::{SdkLogger, SdkLoggerProvider};
use opentelemetry_sdk::Resource;
use opentelemetry_stdout::LogExporter;

use crate::telemetry::error::TelemetryError;

pub struct Logger {
    pub layer: OpenTelemetryTracingBridge<SdkLoggerProvider, SdkLogger>,
    pub provider: SdkLoggerProvider,
}

pub(super) fn _build_logs_provider(
    _config: &TelemetryConfig,
    resource: Resource,
) -> Result<SdkLoggerProvider, TelemetryError> {
    let mut builder = SdkLoggerProvider::builder().with_resource(resource);

    let exporter = LogExporter::default();

    let processor = {
        BatchLogProcessor::builder(exporter)
            .with_batch_config(
                BatchConfigBuilder::default()
                    // TODO: make it configurable
                    .with_max_queue_size(2045)
                    .with_scheduled_delay(Duration::from_secs(5))
                    .with_max_export_batch_size(512)
                    .build(),
            )
            .build()
    };

    builder = builder.with_log_processor(processor);

    Ok(builder.build())
}
