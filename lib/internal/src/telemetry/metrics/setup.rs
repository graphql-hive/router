use hive_router_config::telemetry::{
    metrics::{
        MetricsExporterConfig, MetricsHistogramConfig, MetricsOtlpConfig, MetricsPrometheusConfig,
        MetricsTemporality,
    },
    tracing::OtlpProtocol,
    TelemetryConfig,
};
use opentelemetry_otlp::{
    MetricExporter, Protocol, WithExportConfig, WithHttpConfig, WithTonicConfig,
};
use opentelemetry_prometheus::ResourceSelector;
use opentelemetry_sdk::{metrics::Temporality, Resource};
use opentelemetry_sdk::{
    metrics::{
        periodic_reader_with_async_runtime::PeriodicReader, Aggregation, InstrumentKind,
        MeterProviderBuilder, SdkMeterProvider, Stream,
    },
    runtime,
};
use prometheus::Registry;

use crate::{
    http::normalize_route_path,
    telemetry::{
        error::TelemetryError,
        resolve_string_map,
        utils::{build_metadata, build_tls_config, resolve_value_or_expression},
    },
};

pub struct MetricsSetup {
    pub provider: SdkMeterProvider,
    pub prometheus: Option<PrometheusRuntimeConfig>,
}

pub struct PrometheusRuntimeConfig {
    pub registry: Registry,
    pub port: Option<u16>,
    pub path: String,
}

pub fn build_meter_provider_from_config(
    config: &TelemetryConfig,
    resource: Resource,
) -> Result<Option<MetricsSetup>, TelemetryError> {
    if !config.is_metrics_enabled() {
        return Ok(None);
    }

    build_meter_provider(config, resource).map(Some)
}

fn build_meter_provider(
    config: &TelemetryConfig,
    resource: Resource,
) -> Result<MetricsSetup, TelemetryError> {
    let mut builder = SdkMeterProvider::builder().with_resource(resource);

    builder = setup_view(builder, config)?;
    builder = setup_otlp_readers(builder, config)?;
    let (builder, prometheus_runtime) =
        setup_prometheus_reader(builder, &config.metrics.exporters)?;

    Ok(MetricsSetup {
        provider: builder.build(),
        prometheus: prometheus_runtime,
    })
}

fn setup_view(
    builder: MeterProviderBuilder,
    config: &TelemetryConfig,
) -> Result<MeterProviderBuilder, TelemetryError> {
    let histogram_agg = match config.metrics.instrumentation.common.histogram.clone() {
        MetricsHistogramConfig::Exponential {
            max_size,
            max_scale,
            record_min_max,
        } => Aggregation::Base2ExponentialHistogram {
            max_size,
            max_scale,
            record_min_max,
        },
        MetricsHistogramConfig::Explicit {
            boundaries,
            record_min_max,
        } => Aggregation::ExplicitBucketHistogram {
            record_min_max,
            boundaries,
        },
    };

    Ok(builder.with_view(move |inst| {
        let kind = inst.kind();
        let mut stream = Stream::builder()
            .with_name(inst.name().to_string())
            .with_unit(inst.unit().to_string());

        match kind {
            InstrumentKind::Counter
            | InstrumentKind::UpDownCounter
            | InstrumentKind::ObservableCounter
            | InstrumentKind::ObservableUpDownCounter => {
                stream = stream.with_aggregation(Aggregation::Sum);
            }
            InstrumentKind::Gauge | InstrumentKind::ObservableGauge => {
                stream = stream.with_aggregation(Aggregation::LastValue);
            }
            InstrumentKind::Histogram => {
                stream = stream.with_aggregation(histogram_agg.clone());
            }
        }

        Some(stream.build().expect("Failed to build stream"))
    }))
}

fn setup_otlp_readers(
    mut builder: MeterProviderBuilder,
    config: &TelemetryConfig,
) -> Result<MeterProviderBuilder, TelemetryError> {
    for exporter_config in &config.metrics.exporters {
        match exporter_config {
            MetricsExporterConfig::Otlp(otlp) => {
                builder = builder.with_reader(build_otlp_reader(otlp)?);
            }
            MetricsExporterConfig::Prometheus(_) => {
                // Done in a different step
            }
        }
    }

    Ok(builder)
}

fn build_otlp_reader(
    otlp_config: &MetricsOtlpConfig,
) -> Result<PeriodicReader<MetricExporter>, TelemetryError> {
    ensure_single_otlp_protocol(
        "OTLP metrics exporter",
        &otlp_config.protocol,
        otlp_config.http.is_some(),
        otlp_config.grpc.is_some(),
    )?;

    let endpoint = resolve_value_or_expression(&otlp_config.endpoint, "OTLP metrics endpoint")?;
    let exporter = build_otlp_exporter(otlp_config, endpoint)?;

    // In order to use non-blocking reqwest client,
    // we need to use PeriodicReader from the periodic_reader_with_async_runtime module,
    // and also pass a current-thread runtime.
    // Otherwise it will panic and if we switch to blocking reqwest client,
    // then we will break the hive-console tracing export pipeline.
    Ok(
        PeriodicReader::builder(exporter, runtime::TokioCurrentThread)
            .with_interval(otlp_config.interval)
            .with_timeout(otlp_config.max_export_timeout)
            .build(),
    )
}

fn build_otlp_exporter(
    otlp_config: &MetricsOtlpConfig,
    endpoint: String,
) -> Result<MetricExporter, TelemetryError> {
    match &otlp_config.protocol {
        OtlpProtocol::Grpc => {
            let metadata = otlp_config
                .grpc
                .as_ref()
                .map(|grpc_config| {
                    resolve_string_map(&grpc_config.metadata, "OTLP grpc metadata key")
                })
                .transpose()
                .map(|metadata| metadata.unwrap_or_default())?;

            MetricExporter::builder()
                .with_tonic()
                .with_endpoint(endpoint)
                .with_timeout(otlp_config.max_export_timeout)
                .with_tls_config(build_tls_config(otlp_config.grpc.as_ref().map(|g| &g.tls))?)
                .with_metadata(build_metadata(metadata)?)
                .with_temporality(map_temporality(otlp_config.temporality))
                .build()
                .map_err(|err| TelemetryError::MetricsExporterSetup(err.to_string()))
        }
        OtlpProtocol::Http => {
            let headers = otlp_config
                .http
                .as_ref()
                .map(|http_config| resolve_string_map(&http_config.headers, "OTLP http header key"))
                .transpose()
                .map(|headers| headers.unwrap_or_default())?;

            MetricExporter::builder()
                .with_http()
                .with_endpoint(endpoint)
                .with_timeout(otlp_config.max_export_timeout)
                .with_headers(headers)
                .with_protocol(Protocol::HttpBinary)
                .with_temporality(map_temporality(otlp_config.temporality))
                .build()
                .map_err(|err| TelemetryError::MetricsExporterSetup(err.to_string()))
        }
    }
}

fn setup_prometheus_reader(
    builder: MeterProviderBuilder,
    exporter_configs: &[MetricsExporterConfig],
) -> Result<(MeterProviderBuilder, Option<PrometheusRuntimeConfig>), TelemetryError> {
    let prometheus_exporters: Vec<&Box<MetricsPrometheusConfig>> = exporter_configs
        .iter()
        .filter_map(|cfg| match cfg {
            MetricsExporterConfig::Otlp(_) => None,
            MetricsExporterConfig::Prometheus(prom) => Some(prom),
        })
        .collect();

    if prometheus_exporters.is_empty() {
        return Ok((builder, None));
    }

    if prometheus_exporters.len() > 1 {
        return Err(TelemetryError::MetricsExporterSetup(
            "Multiple Prometheus exporters found".to_string(),
        ));
    }

    let prometheus_config = prometheus_exporters[0];

    let registry = Registry::new();
    if matches!(prometheus_config.port, Some(0)) {
        return Err(TelemetryError::MetricsExporterSetup(
            "Prometheus metrics port must be greater than 0 when set".to_string(),
        ));
    }
    let path = normalize_route_path(&prometheus_config.path);
    let reader = opentelemetry_prometheus::exporter()
        .with_registry(registry.clone())
        .with_resource_selector(ResourceSelector::All)
        .build()
        .map_err(|err| TelemetryError::MetricsExporterSetup(err.to_string()))?;

    Ok((
        builder.with_reader(reader),
        Some(PrometheusRuntimeConfig {
            registry,
            port: prometheus_config.port,
            path,
        }),
    ))
}

fn map_temporality(temporality: MetricsTemporality) -> Temporality {
    match temporality {
        MetricsTemporality::Cumulative => Temporality::Cumulative,
        MetricsTemporality::Delta => Temporality::Delta,
    }
}

fn ensure_single_otlp_protocol(
    name: &str,
    protocol: &OtlpProtocol,
    http_present: bool,
    grpc_present: bool,
) -> Result<(), TelemetryError> {
    match protocol {
        OtlpProtocol::Grpc if http_present => Err(TelemetryError::MetricsExporterSetup(format!(
            "{name} http configuration found while protocol is set to gRPC"
        ))),
        OtlpProtocol::Http if grpc_present => Err(TelemetryError::MetricsExporterSetup(format!(
            "{name} grpc configuration found while protocol is set to HTTP"
        ))),
        _ => Ok(()),
    }
}
