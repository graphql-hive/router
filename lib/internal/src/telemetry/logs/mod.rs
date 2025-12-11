use std::collections::HashMap;
use std::time::Duration;

use hive_router_config::telemetry::TelemetryConfig;
use opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge;
use opentelemetry_otlp::{LogExporter, WithExportConfig, WithTonicConfig};
use opentelemetry_sdk::logs::{BatchConfigBuilder, BatchLogProcessor};
use opentelemetry_sdk::logs::{SdkLogger, SdkLoggerProvider};
use opentelemetry_sdk::Resource;

use crate::telemetry::error::TelemetryError;
use crate::telemetry::utils::build_metadata;

pub struct Logger {
    pub layer: OpenTelemetryTracingBridge<SdkLoggerProvider, SdkLogger>,
    pub provider: SdkLoggerProvider,
}

pub(super) fn build_logs_provider(
    config: &TelemetryConfig,
    resource: Resource,
) -> Result<SdkLoggerProvider, TelemetryError> {
    let mut builder = SdkLoggerProvider::builder().with_resource(resource);

    // OPT-IN

    // TODO: make it configurable
    let exporter_timeout = Duration::from_secs(5);

    let exporter = LogExporter::builder()
        .with_tonic()
        .with_endpoint("http://localhost:4317")
        .with_timeout(exporter_timeout)
        .with_metadata(build_metadata(HashMap::from_iter([(
            "Authorization".to_string(),
            "269dc968-981d-429a-9fcf-af82acc005fd".to_string(),
        )])))
        // .with_tls_config(build_tls_config(grpc_config.tls)?)
        .build()
        .map_err(|e| TelemetryError::LogsExporterSetup(e.to_string()))?;

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
