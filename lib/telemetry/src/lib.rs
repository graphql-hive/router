use hive_router_config::telemetry::TelemetryConfig;
use opentelemetry::trace::TracerProvider;
use opentelemetry::{InstrumentationScope, KeyValue};
use opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge;
use opentelemetry_sdk::{trace::IdGenerator, Resource};
use tracing::Subscriber;
use tracing_subscriber::registry::LookupSpan;

use crate::logs::build_logs_provider;
use crate::traces::build_trace_provider;

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

pub struct OpenTelemetry<Subscriber> {
    pub tracer: Option<traces::Tracer<Subscriber>>,
    // metrics: metrics::MetricsState,
    pub logger: Option<logs::Logger>,
}

impl<S> OpenTelemetry<S> {
    pub fn new_noop() -> OpenTelemetry<S>
    where
        S: Subscriber + for<'span> LookupSpan<'span> + Send + Sync,
    {
        OpenTelemetry {
            tracer: None,
            logger: None,
        }
    }

    pub fn from_config<I>(
        config: &TelemetryConfig,
        id_generator: I,
    ) -> Result<OpenTelemetry<S>, error::TelemetryError>
    where
        S: Subscriber + for<'span> LookupSpan<'span> + Send + Sync,
        I: IdGenerator + 'static,
    {
        // TODO: allow to configure resource attributes through config
        let mut resource_attributes: Vec<_> = Vec::new();

        // TODO: make `service.name` configurable
        resource_attributes.push(KeyValue::new("service.name", config.service.name.clone()));
        let resource = Resource::builder()
            .with_attributes(resource_attributes)
            .build();

        let traces_provider = build_trace_provider(config, id_generator, resource.clone())?;

        // TODO: make those configurable
        let scope = InstrumentationScope::builder("hive-router")
            .with_version("v0")
            .build();

        let tracer = traces_provider.tracer_with_scope(scope);
        let traces_layer = tracing_opentelemetry::layer().with_tracer(tracer);

        let logs_provider = build_logs_provider(config, resource)?;
        let logs_layer = OpenTelemetryTracingBridge::new(&logs_provider);

        Ok(OpenTelemetry {
            tracer: Some(traces::Tracer {
                layer: traces_layer,
                provider: traces_provider,
            }),
            logger: Some(logs::Logger {
                layer: logs_layer,
                provider: logs_provider,
            }),
        })
    }
}
