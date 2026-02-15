//! This module owns the public tracing setup API (`build_otel_layer_from_config`) and
//! a lightweight `TelemetryContext` for explicit propagation without relying on global
//! OpenTelemetry state.
//!
//! It also re-exports the OTEL types used across crates to avoid deep dependency chains.
use hive_router_config::telemetry::TelemetryConfig;
use opentelemetry::trace::TracerProvider;
use opentelemetry::{InstrumentationScope, KeyValue};
use opentelemetry_sdk::{trace::IdGenerator, Resource};
use std::env;
use std::sync::Arc;
use tracing::level_filters::LevelFilter;
use tracing::Subscriber;
use tracing_subscriber::filter::{filter_fn, Targets};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::Layer;

use crate::logging::logger_span::ROUTER_INTERNAL_LOGGER_TARGET;
use crate::telemetry::traces::build_trace_provider;

pub mod error;
pub mod otel;
pub mod traces;
mod utils;

use crate::expressions::{CompileExpression, ExecutableProgram};
use crate::telemetry::error::TelemetryError;
use vrl::core::Value;

pub use otel::opentelemetry::propagation::{
    Injector, TextMapCompositePropagator, TextMapPropagator,
};
pub use otel::opentelemetry::trace::TraceContextExt;
pub use otel::opentelemetry_sdk::trace::{RandomIdGenerator, SdkTracerProvider};
pub use traces::TracerLayer;

/// Context for telemetry operations that doesn't rely on global state.
#[derive(Clone, Debug)]
pub struct TelemetryContext {
    propagator: Option<Arc<TextMapCompositePropagator>>,
}

impl TelemetryContext {
    /// Creates a telemetry context from tracing propagation config
    pub fn from_propagation_config(
        config: &hive_router_config::telemetry::tracing::TracingPropagationConfig,
    ) -> Self {
        use otel::opentelemetry_jaeger_propagator::Propagator as JaegerPropagator;
        use otel::opentelemetry_sdk::propagation::{BaggagePropagator, TraceContextPropagator};
        use otel::opentelemetry_zipkin::Propagator as B3Propagator;

        let mut propagators: Vec<Box<dyn TextMapPropagator + Send + Sync>> = Vec::new();

        if config.trace_context {
            propagators.push(Box::new(TraceContextPropagator::new()));
        }

        if config.baggage {
            propagators.push(Box::new(BaggagePropagator::new()));
        }

        if config.b3 {
            propagators.push(Box::new(B3Propagator::new()));
        }

        if config.jaeger {
            propagators.push(Box::new(JaegerPropagator::new()));
        }

        if propagators.is_empty() {
            return Self { propagator: None };
        }

        Self {
            propagator: Some(Arc::new(TextMapCompositePropagator::new(propagators))),
        }
    }

    pub fn inject_context<I>(&self, injector: &mut I)
    where
        I: Injector,
    {
        use otel::tracing_opentelemetry::OpenTelemetrySpanExt;

        if let Some(propagator) = &self.propagator {
            let current_context = tracing::Span::current().context();
            propagator.inject_context(&current_context, injector);
        }
    }

    pub fn extract_context<E>(&self, extractor: &E) -> otel::opentelemetry::Context
    where
        E: otel::opentelemetry::propagation::Extractor,
    {
        if let Some(propagator) = &self.propagator {
            propagator.extract(extractor)
        } else {
            otel::opentelemetry::Context::new()
        }
    }

    /// Returns true if this context has a propagator configured
    pub fn is_enabled(&self) -> bool {
        self.propagator.is_some()
    }
}

pub fn build_otel_layer_from_config<S, I>(
    config: &TelemetryConfig,
    id_generator: I,
) -> Result<Option<(impl Layer<S> + Send + Sync + 'static, SdkTracerProvider)>, TelemetryError>
where
    S: Subscriber + for<'span> LookupSpan<'span> + Send + Sync + 'static,
    I: IdGenerator + 'static,
{
    if !config.is_tracing_enabled() {
        return Ok(None);
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
    let target_filter = Targets::new()
        .with_target(ROUTER_INTERNAL_LOGGER_TARGET, LevelFilter::OFF)
        .with_default(LevelFilter::INFO);

    let traces_layer = tracing_opentelemetry::layer()
        .with_tracer(tracer)
        .with_tracked_inactivity(false)
        .with_location(false)
        .with_threads(false)
        .with_filter(target_filter)
        // Drop events from tracing macros (info!, error!, etc.),
        // but accept those from span.add_event()
        .with_filter(filter_fn(|metadata| metadata.is_span()));

    Ok(Some((traces_layer, traces_provider)))
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
