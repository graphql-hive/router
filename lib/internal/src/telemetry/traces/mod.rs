use std::collections::HashMap;

use hive_router_config::{
    primitives::value_or_expression::ValueOrExpression,
    telemetry::{
        tracing::{BatchProcessorConfig, OtlpProtocol, TracingExporterConfig},
        TelemetryConfig,
    },
};
use opentelemetry_otlp::{
    Protocol, SpanExporter, WithExportConfig, WithHttpConfig, WithTonicConfig,
};
use opentelemetry_sdk::{
    trace::{
        self, BatchConfigBuilder, BatchSpanProcessor, IdGenerator, Sampler, SdkTracerProvider,
        TracerProviderBuilder,
    },
    Resource,
};
use tracing_opentelemetry::OpenTelemetryLayer;
use vrl::core::Value;

use self::compatibility::HttpCompatibilityExporter;
use crate::{
    expressions::{CompileExpression, ExecutableProgram},
    telemetry::{error::TelemetryError, utils::build_metadata},
};

pub mod compatibility;
pub mod spans;

pub struct Tracer<Subscriber> {
    pub layer: OpenTelemetryLayer<Subscriber, trace::Tracer>,
    pub provider: SdkTracerProvider,
}

pub(super) fn build_trace_provider<I>(
    config: &TelemetryConfig,
    id_generator: I,
    resource: Resource,
) -> Result<SdkTracerProvider, TelemetryError>
where
    I: IdGenerator + 'static,
{
    let base_sampler = Sampler::TraceIdRatioBased(config.tracing.collect.sampling);
    let mut builder = TracerProviderBuilder::default().with_id_generator(id_generator);

    if config.tracing.collect.parent_based_sampler {
        builder = builder.with_sampler(Sampler::ParentBased(Box::new(base_sampler)));
    } else {
        builder = builder.with_sampler(base_sampler);
    }

    builder = builder
        .with_max_events_per_span(config.tracing.collect.max_events_per_span)
        .with_max_attributes_per_span(config.tracing.collect.max_attributes_per_span)
        .with_max_attributes_per_event(config.tracing.collect.max_attributes_per_event)
        .with_max_attributes_per_link(config.tracing.collect.max_attributes_per_link)
        .with_resource(resource);

    Ok(setup_exporters(config, builder)?.build())
}

fn setup_exporters(
    config: &TelemetryConfig,
    mut tracer_provider_builder: TracerProviderBuilder,
) -> Result<TracerProviderBuilder, TelemetryError> {
    let sem_conv_mode = &config.tracing.instrumentation.spans.mode;
    for exporter_config in &config.tracing.exporters {
        let span_processor = match exporter_config {
            TracingExporterConfig::Otlp(otlp_config) => {
                if !otlp_config.enabled {
                    None
                } else {
                    let endpoint = match &otlp_config.endpoint {
                        ValueOrExpression::Value(v) => v.clone(),
                        ValueOrExpression::Expression { expression } => {
                            evaluate_expression_as_string(expression, "OTLP endpoint")?
                        }
                    };

                    let span_exporter = match &otlp_config.protocol {
                        OtlpProtocol::Grpc => {
                            if otlp_config.http.is_some() {
                                return Err(TelemetryError::TracesExporterSetup(
                                    "OTLP http configuration found while protocol is set to gRPC"
                                        .to_string(),
                                ));
                            }

                            let metadata = otlp_config
                                .grpc
                                .as_ref()
                                .map(|grpc_config| {
                                    grpc_config
                                        .metadata
                                        .iter()
                                        .map(|(k, v)| match v {
                                            ValueOrExpression::Value(v) => {
                                                Ok::<_, TelemetryError>((k.clone(), v.clone()))
                                            }
                                            ValueOrExpression::Expression { expression } => {
                                                let value = evaluate_expression_as_string(
                                                    expression,
                                                    &format!("OTLP grpc metadata key '{}'", k),
                                                )?;
                                                Ok::<_, TelemetryError>((k.clone(), value))
                                            }
                                        })
                                        .collect::<Result<HashMap<_, _>, TelemetryError>>()
                                })
                                .transpose()?
                                .unwrap_or_default();

                            let exporter = SpanExporter::builder()
                                .with_tonic()
                                .with_endpoint(endpoint)
                                .with_timeout(otlp_config.batch_processor.max_export_timeout)
                                .with_metadata(build_metadata(metadata))
                                .build()
                                .map_err(|e| TelemetryError::TracesExporterSetup(e.to_string()))?;
                            HttpCompatibilityExporter::new(exporter, sem_conv_mode)
                        }
                        OtlpProtocol::Http => {
                            if otlp_config.grpc.is_some() {
                                return Err(TelemetryError::TracesExporterSetup(
                                    "OTLP grpc configuration found while protocol is set to HTTP"
                                        .to_string(),
                                ));
                            }
                            let headers = otlp_config
                                .http
                                .as_ref()
                                .map(|http_config| {
                                    http_config
                                        .headers
                                        .iter()
                                        .map(|(k, v)| match v {
                                            ValueOrExpression::Value(v) => {
                                                Ok::<_, TelemetryError>((k.clone(), v.clone()))
                                            }
                                            ValueOrExpression::Expression { expression } => {
                                                let value = evaluate_expression_as_string(
                                                    expression,
                                                    &format!("OTLP http header '{}'", k),
                                                )?;
                                                Ok::<_, TelemetryError>((k.clone(), value))
                                            }
                                        })
                                        .collect::<Result<HashMap<_, _>, TelemetryError>>()
                                })
                                .transpose()?
                                .unwrap_or_default();

                            let exporter = SpanExporter::builder()
                                .with_http()
                                .with_endpoint(endpoint)
                                .with_timeout(otlp_config.batch_processor.max_export_timeout)
                                .with_headers(headers)
                                .with_protocol(Protocol::HttpBinary)
                                .build()
                                .map_err(|e| TelemetryError::TracesExporterSetup(e.to_string()))?;

                            HttpCompatibilityExporter::new(exporter, sem_conv_mode)
                        }
                    };

                    Some(build_batched_span_processor(
                        &otlp_config.batch_processor,
                        span_exporter,
                    ))
                }
            }
        };

        if let Some(span_processor) = span_processor {
            tracer_provider_builder = tracer_provider_builder.with_span_processor(span_processor);
        }
    }

    Ok(tracer_provider_builder)
}

fn build_batched_span_processor(
    config: &BatchProcessorConfig,
    exporter: impl trace::SpanExporter + 'static,
) -> BatchSpanProcessor {
    BatchSpanProcessor::builder(exporter)
        .with_batch_config(
            BatchConfigBuilder::default()
                .with_max_concurrent_exports(config.max_concurrent_exports as usize)
                .with_max_export_batch_size(config.max_export_batch_size as usize)
                .with_max_export_timeout(config.max_export_timeout)
                .with_max_queue_size(config.max_queue_size as usize)
                .with_scheduled_delay(config.scheduled_delay)
                .build(),
        )
        .build()
}

fn evaluate_expression_as_string(
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
