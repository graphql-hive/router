use hive_router_config::telemetry::TelemetryConfig;
use opentelemetry::trace::TracerProvider;
use opentelemetry::{InstrumentationScope, KeyValue};
use opentelemetry_sdk::{trace::IdGenerator, Resource};
use std::env;
use tracing::Subscriber;
use tracing_subscriber::registry::LookupSpan;

use crate::telemetry::traces::build_trace_provider;

pub mod error;
pub mod otel;
pub mod traces;
mod utils;

use crate::expressions::{CompileExpression, ExecutableProgram};
use crate::telemetry::error::TelemetryError;
use vrl::core::Value;

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

        let resolved_attributes =
            traces::resolve_string_map(&config.resource.attributes, "resource attribute")?;

        let mut resource_attributes: Vec<_> = resolved_attributes
            .into_iter()
            .map(|(k, v)| KeyValue::new(k, v))
            .collect();

        if !resource_attributes
            .iter()
            .any(|kv| kv.key.as_str() == "service.name")
        {
            resource_attributes.push(KeyValue::new("service.name", "hive-router"));
        }

        let resource = Resource::builder()
            .with_attributes(resource_attributes)
            .build();

        let traces_provider = build_trace_provider(config, id_generator, resource.clone())?;

        let scope = InstrumentationScope::builder("graphql-hive.router")
            .with_version(env!("CARGO_PKG_VERSION"))
            .build();

        let tracer = traces_provider.tracer_with_scope(scope);
        let traces_layer = tracing_opentelemetry::layer()
            .with_tracer(tracer)
            .with_tracked_inactivity(false)
            .with_location(false)
            .with_threads(false);

        Ok(OpenTelemetry {
            tracer: Some(traces::Tracer {
                layer: traces_layer,
                provider: traces_provider,
            }),
        })
    }
}

pub fn evaluate_expression_as_string(
    expression: &str,
    context: &str,
) -> Result<String, TelemetryError> {
    Ok(expression
        // compile
        .compile_expression(None)
        .map_err(|e| {
            TelemetryError::TracesExporterSetup(format!(
                "Failed to compile {} expression: {}",
                context, e
            ))
        })?
        // execute
        .execute(Value::Null) // no input context as we are in setup phase
        .map_err(|e| {
            TelemetryError::TracesExporterSetup(format!(
                "Failed to execute {} expression: {}",
                context, e
            ))
        })?
        // coerce
        .as_str()
        .ok_or_else(|| {
            TelemetryError::TracesExporterSetup(format!(
                "{} expression must return a string",
                context
            ))
        })?
        .to_string())
}

pub fn resolve_value_or_expression(
    value_or_expr: &hive_router_config::primitives::value_or_expression::ValueOrExpression<String>,
    context: &str,
) -> Result<String, TelemetryError> {
    match value_or_expr {
        hive_router_config::primitives::value_or_expression::ValueOrExpression::Value(v) => {
            Ok(v.clone())
        }
        hive_router_config::primitives::value_or_expression::ValueOrExpression::Expression {
            expression,
        } => evaluate_expression_as_string(expression, context),
    }
}
