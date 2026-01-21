use std::collections::HashMap;

use hive_router_config::{
    primitives::value_or_expression::ValueOrExpression,
    telemetry::{
        hive::{is_slug_target_ref, is_uuid_target_ref, HiveTelemetryConfig},
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
        SpanProcessor, TracerProviderBuilder,
    },
    Resource,
};
use tracing_opentelemetry::OpenTelemetryLayer;

use self::compatibility::HttpCompatibilityExporter;
use self::filtering_exporter::FilteringSpanExporter;
use crate::telemetry::{
    error::TelemetryError,
    resolve_value_or_expression,
    traces::hive_console_exporter::HiveConsoleExporter,
    utils::{build_metadata, build_tls_config},
};

pub use control::{disabled_span, is_level_enabled, is_tracing_enabled, set_tracing_enabled};

pub mod compatibility;
pub mod control;
pub mod filtering_exporter;
pub mod hive_console_exporter;
pub mod spans;
pub mod stdout_exporter;

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
    let mut builder = TracerProviderBuilder::default()
        .with_id_generator(id_generator)
        .with_resource(resource.clone());

    if config.tracing.collect.parent_based_sampler {
        builder = builder.with_sampler(Sampler::ParentBased(Box::new(base_sampler)));
    } else {
        builder = builder.with_sampler(base_sampler);
    }

    builder = builder
        .with_max_events_per_span(config.tracing.collect.max_events_per_span)
        .with_max_attributes_per_span(config.tracing.collect.max_attributes_per_span)
        .with_max_attributes_per_event(config.tracing.collect.max_attributes_per_event)
        .with_max_attributes_per_link(config.tracing.collect.max_attributes_per_link);

    Ok(setup_exporters(config, resource, builder)?.build())
}

fn setup_exporters(
    config: &TelemetryConfig,
    resource: Resource,
    mut tracer_provider_builder: TracerProviderBuilder,
) -> Result<TracerProviderBuilder, TelemetryError> {
    let sem_conv_mode = &config.tracing.instrumentation.spans.mode;
    for exporter_config in &config.tracing.exporters {
        match exporter_config {
            TracingExporterConfig::Otlp(otlp_config) => {
                if !otlp_config.enabled {
                    continue;
                }

                ensure_single_protocol_config(
                    "OTLP exporter",
                    &otlp_config.protocol,
                    otlp_config.http.is_some(),
                    otlp_config.grpc.is_some(),
                )?;
                let endpoint = resolve_value_or_expression(&otlp_config.endpoint, "OTLP endpoint")?;

                let exporter = match &otlp_config.protocol {
                    OtlpProtocol::Grpc => {
                        let metadata = otlp_config
                            .grpc
                            .as_ref()
                            .map(|grpc_config| {
                                resolve_string_map(&grpc_config.metadata, "OTLP grpc metadata key")
                            })
                            .transpose()?
                            .unwrap_or_default();

                        SpanExporter::builder()
                            .with_tonic()
                            .with_endpoint(endpoint)
                            .with_timeout(otlp_config.batch_processor.max_export_timeout)
                            .with_tls_config(build_tls_config(
                                otlp_config.grpc.as_ref().map(|g| &g.tls),
                            )?)
                            .with_metadata(build_metadata(metadata)?)
                            .build()
                    }
                    OtlpProtocol::Http => {
                        let headers = otlp_config
                            .http
                            .as_ref()
                            .map(|http_config| {
                                resolve_string_map(&http_config.headers, "OTLP http header key")
                            })
                            .transpose()?
                            .unwrap_or_default();

                        SpanExporter::builder()
                            .with_http()
                            .with_endpoint(endpoint)
                            .with_timeout(otlp_config.batch_processor.max_export_timeout)
                            .with_headers(headers)
                            .with_protocol(Protocol::HttpBinary)
                            .build()
                    }
                }
                .map_err(|e| TelemetryError::TracesExporterSetup(e.to_string()))?;

                tracer_provider_builder =
                    tracer_provider_builder.with_span_processor(build_batched_span_processor(
                        &otlp_config.batch_processor,
                        &resource,
                        HttpCompatibilityExporter::new(exporter, sem_conv_mode),
                    ));
            }
            TracingExporterConfig::Stdout(stdout_config) => {
                if !stdout_config.enabled {
                    continue;
                }

                tracer_provider_builder =
                    tracer_provider_builder.with_span_processor(build_batched_span_processor(
                        &stdout_config.batch_processor,
                        &resource,
                        HttpCompatibilityExporter::new(
                            stdout_exporter::StdoutExporter::new(),
                            sem_conv_mode,
                        ),
                    ));
            }
        }
    }

    if let Some(hive_config) = &config.hive {
        if hive_config.tracing.enabled {
            tracer_provider_builder =
                setup_hive_exporter(hive_config, &resource, tracer_provider_builder)?;
        }
    }

    Ok(tracer_provider_builder)
}

fn build_batched_span_processor(
    config: &BatchProcessorConfig,
    resource: &Resource,
    exporter: impl trace::SpanExporter + 'static,
) -> BatchSpanProcessor {
    let mut processor = BatchSpanProcessor::builder(FilteringSpanExporter::new(exporter))
        .with_batch_config(
            BatchConfigBuilder::default()
                .with_max_concurrent_exports(config.max_concurrent_exports as usize)
                .with_max_export_batch_size(config.max_export_batch_size as usize)
                .with_max_export_timeout(config.max_export_timeout)
                .with_max_queue_size(config.max_queue_size as usize)
                .with_scheduled_delay(config.scheduled_delay)
                .build(),
        )
        .build();

    processor.set_resource(resource);

    processor
}

fn setup_hive_exporter(
    config: &HiveTelemetryConfig,
    resource: &Resource,
    tracer_provider_builder: TracerProviderBuilder,
) -> Result<TracerProviderBuilder, TelemetryError> {
    let endpoint = resolve_value_or_expression(&config.endpoint, "Hive Tracing endpoint")?;
    let token = match &config.token {
        Some(t) => resolve_value_or_expression(t, "Hive Telemetry token")?,
        None => {
            return Err(TelemetryError::TracesExporterSetup(
                "Hive Tracing token is required but not provided".to_string(),
            ))
        }
    };
    let target = match &config.target {
        Some(t) => resolve_value_or_expression(t, "Hive Telemetry target")?,
        None => {
            return Err(TelemetryError::TracesExporterSetup(
                "Hive Tracing target is required but not provided".to_string(),
            ))
        }
    };

    if !is_uuid_target_ref(&token) && !is_slug_target_ref(&token) {
        return Err(TelemetryError::TracesExporterSetup(format!(
            "Invalid Hive Tracing target format: '{}'. It must be either in slug format '$organizationSlug/$projectSlug/$targetSlug' or UUID format 'a0f4c605-6541-4350-8cfe-b31f21a4bf80'",
            target
        )));
    }

    ensure_single_protocol_config(
        "Hive Tracing",
        &config.tracing.protocol,
        config.tracing.http.is_some(),
        config.tracing.grpc.is_some(),
    )?;

    let exporter = match &config.tracing.protocol {
        OtlpProtocol::Grpc => {
            let mut metadata = config
                .tracing
                .grpc
                .as_ref()
                .map(|grpc_config| {
                    resolve_string_map(&grpc_config.metadata, "Hive Tracing grpc metadata key")
                })
                .transpose()?
                .unwrap_or_default();

            metadata.insert("authorization".to_string(), token);
            metadata.insert("x-hive-target-ref".to_string(), target);

            SpanExporter::builder()
                .with_tonic()
                .with_endpoint(endpoint)
                .with_timeout(config.tracing.batch_processor.max_export_timeout)
                .with_metadata(build_metadata(metadata)?)
                .with_tls_config(build_tls_config(
                    config.tracing.grpc.as_ref().map(|g| &g.tls),
                )?)
                .build()
        }
        OtlpProtocol::Http => {
            let mut headers = config
                .tracing
                .http
                .as_ref()
                .map(|http_config| {
                    resolve_string_map(&http_config.headers, "Hive Tracing http header key")
                })
                .transpose()?
                .unwrap_or_default();

            headers.insert("authorization".to_string(), format!("Bearer {}", token));
            headers.insert("x-hive-target-ref".to_string(), target);

            SpanExporter::builder()
                .with_http()
                .with_endpoint(endpoint)
                .with_timeout(config.tracing.batch_processor.max_export_timeout)
                .with_headers(headers)
                .with_protocol(Protocol::HttpBinary)
                .build()
        }
    }
    .map_err(|e| TelemetryError::TracesExporterSetup(e.to_string()))?;

    Ok(
        tracer_provider_builder.with_span_processor(build_batched_span_processor(
            &config.tracing.batch_processor,
            &resource,
            HiveConsoleExporter::new(exporter),
        )),
    )
}

pub(crate) fn resolve_string_map(
    map: &HashMap<String, ValueOrExpression<String>>,
    context_prefix: &str,
) -> Result<HashMap<String, String>, TelemetryError> {
    map.iter()
        .map(|(k, v)| {
            let value = resolve_value_or_expression(v, &format!("{} '{}'", context_prefix, k))?;
            Ok((k.clone(), value))
        })
        .collect()
}

fn ensure_single_protocol_config(
    name: &str,
    protocol: &OtlpProtocol,
    http_present: bool,
    grpc_present: bool,
) -> Result<(), TelemetryError> {
    match protocol {
        OtlpProtocol::Grpc if http_present => Err(TelemetryError::TracesExporterSetup(format!(
            "{name} http configuration found while protocol is set to gRPC"
        ))),
        OtlpProtocol::Http if grpc_present => Err(TelemetryError::TracesExporterSetup(format!(
            "{name} grpc configuration found while protocol is set to HTTP"
        ))),
        _ => Ok(()),
    }
}
