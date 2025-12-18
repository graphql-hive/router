use hive_router_config::telemetry::TelemetryConfig;
use opentelemetry::trace::TracerProvider;
use opentelemetry::{InstrumentationScope, KeyValue};
// use opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge;
use opentelemetry_sdk::{trace::IdGenerator, Resource};
use tracing::Subscriber;
use tracing_subscriber::registry::LookupSpan;

// use crate::telemetry::logs::build_logs_provider;
use crate::telemetry::traces::build_trace_provider;

mod error;
pub mod logs;
pub mod metrics;
pub mod otel;
pub mod traces;
mod utils;

// docker run -p 4316:8080 -p 4317:4317 -p 4318:4318 docker.hyperdx.io/hyperdx/hyperdx-all-in-one
// Send OpenTelemetry data via:
//   http/protobuf: OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4318
//   gRPC: OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4317

pub struct OpenTelemetry {
    pub tracer: Option<traces::Tracer>,
    // metrics: metrics::MetricsState,
    pub logger: Option<logs::Logger>,
}

impl OpenTelemetry {
    pub fn new_noop() -> OpenTelemetry {
        OpenTelemetry {
            tracer: None,
            logger: None,
        }
    }

    pub fn from_config<I>(
        config: &TelemetryConfig,
        id_generator: I,
    ) -> Result<OpenTelemetry, error::TelemetryError>
    where
        I: IdGenerator + 'static,
    {
        if !config.is_tracing_enabled() {
            return Ok(OpenTelemetry::new_noop());
        }

        // TODO: allow to configure resource attributes through config
        // TODO: make `service.name` configurable
        let resource_attributes: Vec<_> =
            vec![KeyValue::new("service.name", config.service.name.clone())];

        let resource = Resource::builder()
            .with_attributes(resource_attributes)
            .build();

        let traces_provider = build_trace_provider(config, id_generator, resource.clone())?;

        // TODO: make those configurable
        // let scope = InstrumentationScope::builder("hive-router")
        //     .with_version("v0")
        //     .build();

        // let tracer = traces_provider.tracer_with_scope(scope);

        // let logs_provider = build_logs_provider(config, resource)?;
        // let logs_layer = OpenTelemetryTracingBridge::new(&logs_provider);

        Ok(OpenTelemetry {
            tracer: Some(traces::Tracer {
                provider: traces_provider,
            }),
            logger: None,
        })
    }
}
