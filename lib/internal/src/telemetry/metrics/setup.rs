use std::collections::{HashMap, HashSet};

use hive_router_config::{
    primitives::toggle::ToggleWith,
    telemetry::{
        metrics::{
            MetricsExporterConfig, MetricsHistogramConfig, MetricsOtlpConfig,
            MetricsPrometheusConfig, MetricsTemporality,
        },
        tracing::OtlpProtocol,
        TelemetryConfig,
    },
};
use opentelemetry::Key;
use opentelemetry_otlp::{
    MetricExporter, Protocol, WithExportConfig, WithHttpConfig, WithTonicConfig,
};
use opentelemetry_prometheus::ResourceSelector;
#[cfg(feature = "noop_otlp_exporter")]
use opentelemetry_sdk::{error::OTelSdkResult, metrics::data::ResourceMetrics};
use opentelemetry_sdk::{metrics::Temporality, Resource};
use opentelemetry_sdk::{
    metrics::{
        exporter::PushMetricExporter, periodic_reader_with_async_runtime::PeriodicReader,
        Aggregation, InstrumentKind, MeterProviderBuilder, SdkMeterProvider, Stream,
    },
    runtime,
};
use prometheus::Registry;
use tracing::warn;

use crate::{
    http::normalize_route_path,
    telemetry::{
        error::TelemetryError,
        metrics::catalog::{all_metric_names, labels_for},
        resolve_string_map,
        utils::{build_metadata, build_tls_config, resolve_value_or_expression},
    },
};

enum InstrumentRule {
    Disabled,
    Filtered(Vec<Key>),
}

fn build_instrument_rules(
    config: &TelemetryConfig,
) -> Result<HashMap<String, InstrumentRule>, TelemetryError> {
    // Build rules only for metrics that differ from default behavior.
    // Metrics that stay at defaults are not added to this map.
    let mut rules = HashMap::new();

    for (metric_name, toggle) in &config.metrics.instrumentation.instruments {
        // Metric names are validated strictly: unknown names fail startup.
        let Some(default_labels) = labels_for(metric_name.as_str()) else {
            let mut valid_metrics = all_metric_names();
            valid_metrics.sort_unstable();
            return Err(TelemetryError::MetricsExporterSetup(format!(
                "Unknown metric in metrics.instrumentation.instruments: {metric_name}. Valid metrics: {}",
                valid_metrics.join(", ")
            )));
        };

        match toggle {
            ToggleWith::Disabled => {
                rules.insert(metric_name.clone(), InstrumentRule::Disabled);
            }
            ToggleWith::Enabled(instrument_config) => {
                if instrument_config.attributes.is_empty() {
                    continue;
                }

                let mut allowed_labels: HashSet<&str> = default_labels.iter().copied().collect();

                // Unknown attribute keys log a warning and are ignored.
                for (attribute_name, keep) in &instrument_config.attributes {
                    if !default_labels.contains(&attribute_name.as_str()) {
                        let valid_labels = default_labels.join(", ");
                        warn!(
                            metric = metric_name,
                            attribute = attribute_name,
                            valid_labels = %valid_labels,
                            "Unknown metric attribute in metrics.instrumentation.instruments, ignoring"
                        );
                        continue;
                    }

                    if !keep {
                        allowed_labels.remove(attribute_name.as_str());
                    }
                }

                // No effective label change means no custom rule is needed.
                if allowed_labels.len() == default_labels.len() {
                    continue;
                }

                let filtered_labels = default_labels
                    .iter()
                    .filter(|label| allowed_labels.contains(**label))
                    .map(|label| Key::new(*label))
                    .collect();

                rules.insert(
                    metric_name.clone(),
                    InstrumentRule::Filtered(filtered_labels),
                );
            }
        }
    }

    Ok(rules)
}

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
    let instrument_rules = build_instrument_rules(config)?;
    let histogram_config = config.metrics.instrumentation.common.histogram.clone();
    validate_histogram_config(&histogram_config)?;

    Ok(builder.with_view(move |inst| {
        let kind = inst.kind();
        let mut stream = Stream::builder()
            .with_name(inst.name().to_string())
            .with_unit(inst.unit().to_string());

        if matches!(
            instrument_rules.get(inst.name()),
            Some(InstrumentRule::Disabled)
        ) {
            stream = stream.with_aggregation(Aggregation::Drop);
        } else {
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
                    let histogram_agg =
                        histogram_aggregation_for_unit(&histogram_config, inst.name(), inst.unit())
                            .unwrap_or_else(|err| panic!("{err}"));
                    stream = stream.with_aggregation(histogram_agg);
                }
            }
        }

        if let Some(InstrumentRule::Filtered(filtered_labels)) = instrument_rules.get(inst.name()) {
            stream = stream.with_allowed_attribute_keys(filtered_labels.clone());
        }

        Some(stream.build().expect("Failed to build stream"))
    }))
}

fn validate_histogram_config(config: &MetricsHistogramConfig) -> Result<(), TelemetryError> {
    let MetricsHistogramConfig::Explicit { seconds, bytes } = config else {
        return Ok(());
    };

    let seconds_buckets = seconds
        .resolve_seconds_buckets()
        .map_err(TelemetryError::MetricsExporterSetup)?;
    let bytes_buckets = bytes
        .resolve_bytes_buckets()
        .map_err(TelemetryError::MetricsExporterSetup)?;

    validate_explicit_histogram_buckets("seconds", &seconds_buckets)?;
    validate_explicit_histogram_buckets("bytes", &bytes_buckets)?;
    Ok(())
}

fn validate_explicit_histogram_buckets(
    bucket_set_name: &str,
    buckets: &[f64],
) -> Result<(), TelemetryError> {
    if buckets.is_empty() {
        return Err(TelemetryError::MetricsExporterSetup(format!(
            "telemetry.metrics.instrumentation.common.histogram.{bucket_set_name}.buckets must not be empty"
        )));
    }

    let mut previous: Option<f64> = None;
    for value in buckets {
        if !value.is_finite() {
            return Err(TelemetryError::MetricsExporterSetup(format!(
                "telemetry.metrics.instrumentation.common.histogram.{bucket_set_name}.buckets must contain only finite values"
            )));
        }

        if *value < 0.0 {
            return Err(TelemetryError::MetricsExporterSetup(format!(
                "telemetry.metrics.instrumentation.common.histogram.{bucket_set_name}.buckets must contain only non-negative values"
            )));
        }

        if let Some(previous) = previous {
            if *value <= previous {
                return Err(TelemetryError::MetricsExporterSetup(format!(
                    "telemetry.metrics.instrumentation.common.histogram.{bucket_set_name}.buckets must be strictly increasing"
                )));
            }
        }

        previous = Some(*value);
    }

    Ok(())
}

fn histogram_aggregation_for_unit(
    histogram_config: &MetricsHistogramConfig,
    instrument_name: &str,
    instrument_unit: &str,
) -> Result<Aggregation, TelemetryError> {
    match histogram_config {
        MetricsHistogramConfig::Exponential {
            max_size,
            max_scale,
            record_min_max,
        } => Ok(Aggregation::Base2ExponentialHistogram {
            max_size: *max_size,
            max_scale: *max_scale,
            record_min_max: *record_min_max,
        }),
        MetricsHistogramConfig::Explicit { seconds, bytes } => {
            let buckets = match instrument_unit {
                "s" => seconds
                    .resolve_seconds_buckets()
                    .map_err(TelemetryError::MetricsExporterSetup)?,
                "By" => bytes
                    .resolve_bytes_buckets()
                    .map_err(TelemetryError::MetricsExporterSetup)?,
                _ => {
                    return Err(TelemetryError::MetricsExporterSetup(format!(
                        "Unsupported histogram unit '{instrument_unit}' for instrument '{instrument_name}' in explicit histogram aggregation. Supported units: s, By"
                    )));
                }
            };

            Ok(explicit_histogram_aggregation(
                buckets,
                match instrument_unit {
                    "s" => seconds.record_min_max,
                    "By" => bytes.record_min_max,
                    _ => unreachable!("unsupported instrument unit already handled"),
                },
            ))
        }
    }
}

fn explicit_histogram_aggregation(buckets: Vec<f64>, record_min_max: bool) -> Aggregation {
    Aggregation::ExplicitBucketHistogram {
        boundaries: buckets,
        record_min_max,
    }
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
) -> Result<PeriodicReader<impl PushMetricExporter>, TelemetryError> {
    ensure_single_otlp_protocol(
        "OTLP metrics exporter",
        &otlp_config.protocol,
        otlp_config.http.is_some(),
        otlp_config.grpc.is_some(),
    )?;

    let endpoint = resolve_value_or_expression(&otlp_config.endpoint, "OTLP metrics endpoint")?;
    let exporter = build_otlp_exporter(otlp_config, endpoint)?;

    #[cfg(feature = "noop_otlp_exporter")]
    let exporter = {
        let _ = exporter;
        NoopExporter {}
    };

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

#[cfg(feature = "noop_otlp_exporter")]
struct NoopExporter {}

#[cfg(feature = "noop_otlp_exporter")]
impl PushMetricExporter for NoopExporter {
    async fn export(&self, _metrics: &ResourceMetrics) -> OTelSdkResult {
        Ok(())
    }

    fn force_flush(&self) -> OTelSdkResult {
        // this component is stateless
        Ok(())
    }

    fn shutdown(&self) -> OTelSdkResult {
        Ok(())
    }

    fn shutdown_with_timeout(&self, _timeout: std::time::Duration) -> OTelSdkResult {
        Ok(())
    }

    fn temporality(&self) -> Temporality {
        Temporality::Cumulative
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use hive_router_config::{
        primitives::toggle::ToggleWith,
        telemetry::{metrics::InstrumentConfig, TelemetryConfig},
    };

    use crate::telemetry::metrics::catalog::{labels, names};

    use super::{build_instrument_rules, InstrumentRule};

    #[test]
    fn fail_on_unknown_metric() {
        let mut config = TelemetryConfig::default();
        config
            .metrics
            .instrumentation
            .instruments
            .insert("unknown.metric".to_string(), ToggleWith::Disabled);

        let err_msg = match build_instrument_rules(&config) {
            Ok(_) => panic!("should fail for unknown metric"),
            Err(err) => err.to_string(),
        };

        assert!(err_msg.contains("Unknown metric in metrics.instrumentation.instruments"));
        assert!(err_msg.contains("unknown.metric"));
    }

    #[test]
    fn ignores_known_disable_label() {
        let mut config = TelemetryConfig::default();
        config.metrics.instrumentation.instruments.insert(
            names::HTTP_SERVER_REQUEST_DURATION.to_string(),
            ToggleWith::Enabled(InstrumentConfig {
                attributes: HashMap::from([(labels::HTTP_ROUTE.to_string(), false)]),
            }),
        );

        let rules = build_instrument_rules(&config).expect("config should be valid");
        let rule = rules
            .get(names::HTTP_SERVER_REQUEST_DURATION)
            .expect("rule should exist");

        let InstrumentRule::Filtered(allowed) = rule else {
            panic!("expected filtered rule")
        };

        let allowed: Vec<&str> = allowed.iter().map(|key| key.as_str()).collect();

        assert!(allowed.contains(&labels::HTTP_REQUEST_METHOD));
        assert!(allowed.contains(&labels::HTTP_RESPONSE_STATUS_CODE));
        assert!(!allowed.contains(&labels::HTTP_ROUTE));
    }

    #[test]
    fn ignores_unknown_label() {
        let mut config = TelemetryConfig::default();
        config.metrics.instrumentation.instruments.insert(
            names::HTTP_SERVER_REQUEST_DURATION.to_string(),
            ToggleWith::Enabled(InstrumentConfig {
                attributes: HashMap::from([("unknown.label".to_string(), true)]),
            }),
        );

        let rules = build_instrument_rules(&config).expect("config should be valid");
        let rule = rules.get(names::HTTP_SERVER_REQUEST_DURATION);

        // If a rule does not exist, it means that the metric gets all labels
        assert!(rule.is_none(), "rule should not exist");
    }
}
