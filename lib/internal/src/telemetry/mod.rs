use hive_router_config::telemetry::TelemetryConfig;
use opentelemetry::trace::TracerProvider;
use opentelemetry::{InstrumentationScope, KeyValue};
use opentelemetry_sdk::{trace::IdGenerator, Resource};
use tracing::Subscriber;
use tracing_subscriber::registry::LookupSpan;

use crate::telemetry::traces::build_trace_provider;

mod error;
pub mod otel;
pub mod traces;
mod utils;

pub struct OpenTelemetry<Subscriber> {
    pub tracer: Option<traces::Tracer<Subscriber>>,
    // logs and metrics can be added here later
}

impl<S> OpenTelemetry<S> {
    pub fn new_noop() -> OpenTelemetry<S>
    where
        S: Subscriber + for<'span> LookupSpan<'span> + Send + Sync,
    {
        OpenTelemetry { tracer: None }
    }

    pub fn from_config<I>(
        config: &TelemetryConfig,
        id_generator: I,
    ) -> Result<OpenTelemetry<S>, error::TelemetryError>
    where
        S: Subscriber + for<'span> LookupSpan<'span> + Send + Sync,
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
        let scope = InstrumentationScope::builder("hive-router")
            .with_version("v0")
            .build();

        let tracer = traces_provider.tracer_with_scope(scope);
        let traces_layer = tracing_opentelemetry::layer().with_tracer(tracer);

        Ok(OpenTelemetry {
            tracer: Some(traces::Tracer {
                layer: traces_layer,
                provider: traces_provider,
            }),
        })
    }
}
