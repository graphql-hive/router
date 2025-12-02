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

use crate::{error::TelemetryError, utils::build_metadata};

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
    println!("Setting up tracing exporters...");
    for exporter_config in &config.tracing.exporters {
        let span_processor = match exporter_config {
            TracingExporterConfig::Otlp(otlp_config) => {
                println!("Setting up OTLP tracing exporter...");
                if !otlp_config.enabled {
                    println!("OTLP tracing exporter is disabled.");
                    None
                } else {
                    println!("OTLP tracing exporter is enabled.");
                    let endpoint = match &otlp_config.endpoint {
                        ValueOrExpression::Value(v) => v.clone(),
                        ValueOrExpression::Expression { .. } => {
                            return Err(TelemetryError::MetricsExporterSetup(
                                "OTLP endpoint expressions are not supported yet".to_string(),
                            ));
                        }
                    };

                    println!("OTLP tracing endpoint: {}", endpoint);

                    let span_exporter = match &otlp_config.protocol {
                        OtlpProtocol::Grpc => {
                            println!("OTLP tracing protocol: gRPC");
                            if otlp_config.http.is_some() {
                                return Err(TelemetryError::SpanExporterSetup(
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
                                                Ok((k.clone(), v.clone()))
                                            }
                                            ValueOrExpression::Expression { .. } => {
                                                Err(TelemetryError::MetricsExporterSetup(
                                                    "OTLP header expressions are not supported yet"
                                                        .to_string(),
                                                ))
                                            }
                                        })
                                        .collect::<Result<HashMap<_, _>, _>>()
                                })
                                .transpose()?
                                .unwrap_or_default();

                            println!("OTLP tracing metadata: {:?}", metadata);

                            SpanExporter::builder()
                                .with_tonic()
                                .with_endpoint(endpoint)
                                .with_timeout(otlp_config.batch_processor.max_export_timeout)
                                .with_metadata(build_metadata(metadata))
                                .build()
                                .map_err(|e| TelemetryError::SpanExporterSetup(e.to_string()))?
                        }
                        OtlpProtocol::Http => {
                            println!("OTLP tracing protocol: HTTP");
                            if otlp_config.grpc.is_some() {
                                return Err(TelemetryError::SpanExporterSetup(
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
                                                Ok((k.clone(), v.clone()))
                                            }
                                            ValueOrExpression::Expression { .. } => {
                                                Err(TelemetryError::MetricsExporterSetup(
                                                    "OTLP header expressions are not supported yet"
                                                        .to_string(),
                                                ))
                                            }
                                        })
                                        .collect::<Result<HashMap<_, _>, _>>()
                                })
                                .transpose()?
                                .unwrap_or_default();

                            println!("OTLP tracing headers: {:?}", headers);

                            SpanExporter::builder()
                                .with_http()
                                .with_endpoint(endpoint)
                                .with_timeout(otlp_config.batch_processor.max_export_timeout)
                                .with_headers(headers)
                                .with_protocol(Protocol::HttpBinary)
                                .build()
                                .map_err(|e| TelemetryError::SpanExporterSetup(e.to_string()))?
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
            println!("Adding span processor to tracer provider...");
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
