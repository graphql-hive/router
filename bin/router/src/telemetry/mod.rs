//! This module owns the public tracing setup API (`build_otel_layer_from_config`) and
//! a lightweight `TelemetryContext` for explicit propagation without relying on global
//! OpenTelemetry state.
//!
//! It also re-exports the OTEL types used across crates to avoid deep dependency chains.

pub mod error;
pub mod logging;
pub mod metrics;
pub mod propagation;
pub mod traces;
pub mod utils;

use crate::config::telemetry::tracing::TracingPropagationConfig;
use crate::config::telemetry::TelemetryConfig;
use crate::config::{
    log::{LogFormat, LoggingConfig},
    HiveRouterConfig,
};
use crate::telemetry::{
    error::TelemetryError,
    logging::{
        format_json::RouterJsonFormat,
        format_text::RouterTextFormat,
        targets,
        utils::{create_ignore_otel_filter, create_targets_filter, DynLayer},
    },
    metrics::{build_meter_provider_from_config, PrometheusRuntimeConfig},
    traces::set_tracing_enabled,
    utils::http::normalize_route_path,
};
use opentelemetry::metrics::Meter;
use opentelemetry::propagation::{Injector, TextMapCompositePropagator, TextMapPropagator};
use opentelemetry::trace::TracerProvider;
use opentelemetry::{InstrumentationScope, KeyValue};
use opentelemetry_sdk::{trace::IdGenerator, Resource};
use std::env;
use std::sync::Arc;
use tracing::{warn, Subscriber};
use tracing_subscriber::filter::filter_fn;
use tracing_subscriber::registry::LookupSpan;

use crate::telemetry::logging::request_id::RequestIdentifierExtractor;
use crate::telemetry::metrics::Metrics;
use crate::telemetry::propagation::HeaderMapInjector;
use crate::telemetry::traces::build_trace_provider;

use ntex::web::{self};
use ntex::web::{App, HttpResponse, HttpServer};
use opentelemetry::{
    global::{set_meter_provider, set_tracer_provider},
    metrics::MeterProvider,
    propagation::Extractor,
};
use opentelemetry_sdk::{
    metrics::SdkMeterProvider,
    trace::{RandomIdGenerator, SdkTracerProvider},
};
use prometheus::{Encoder, TextEncoder};
use std::{io::IsTerminal, str::FromStr, sync::Mutex};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{filter::ParseError, fmt::time::UtcTime, Registry};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use tracing_subscriber::{EnvFilter, Layer};
use utils::resolve_string_map;

pub struct HeaderExtractor<'a>(pub &'a ntex::http::HeaderMap);

impl Extractor for HeaderExtractor<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(|value| value.to_str().ok())
    }

    fn keys(&self) -> Vec<&str> {
        self.0
            .keys()
            .map(|value| value.as_str())
            .collect::<Vec<_>>()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TelemetryInitError {
    #[error(transparent)]
    TelemetryError(#[from] TelemetryError),
    #[error("failed to initialize prometheus server: {0}")]
    PrometheusServerError(#[from] std::io::Error),
    #[error("failed to initialize env-filter logger: {0}")]
    EnvFilter(#[from] tracing_subscriber::filter::ParseError),
}

pub struct Telemetry {
    pub traces_provider: Option<SdkTracerProvider>,
    pub metrics_provider: Option<SdkMeterProvider>,
    pub prometheus: Option<PrometheusRuntime>,
    pub context: TelemetryContext,
    pub logging_writer_guard: tracing_appender::non_blocking::WorkerGuard,
}

pub enum PrometheusRuntime {
    Attached(PrometheusAttached),
    Detached {
        registry: prometheus::Registry,
        endpoint: String,
        server: ntex::server::Server,
        handle: Mutex<tokio::task::JoinHandle<()>>,
    },
}

#[derive(Clone)]
pub struct PrometheusAttached {
    pub registry: prometheus::Registry,
    pub endpoint: String,
}

impl PrometheusRuntime {
    async fn shutdown(&self) {
        match self {
            PrometheusRuntime::Attached { .. } => {}
            PrometheusRuntime::Detached { server, .. } => {
                server.stop(true).await;
            }
        }
    }

    pub fn to_attached(&self) -> Option<PrometheusAttached> {
        match self {
            PrometheusRuntime::Attached(attached) => Some(attached.clone()),
            _ => None,
        }
    }
}

impl Telemetry {
    /// Sets up the global tracing subscriber including logging and OpenTelemetry.
    pub fn init_global(config: &HiveRouterConfig) -> Result<Self, TelemetryInitError> {
        let id_generator = RandomIdGenerator::default();
        let resource = build_resource(&config.telemetry)?;
        let scope = build_scope();
        let otel_layer_result = build_otel_layer_from_config(
            &config.telemetry,
            id_generator,
            scope.clone(),
            resource.clone(),
        )?;
        let metrics_result = build_meter_provider_from_config(&config.telemetry, resource)?;

        let (otel_layer, tracer_provider) = if let Some((layer, provider)) = otel_layer_result {
            set_tracing_enabled(true);
            set_tracer_provider(provider.clone());
            (Some(layer), Some(provider))
        } else {
            set_tracing_enabled(false);
            (None, None)
        };

        let (metrics_provider, prometheus_config) = if let Some(metrics_setup) = metrics_result {
            set_meter_provider(metrics_setup.provider.clone());
            (Some(metrics_setup.provider), metrics_setup.prometheus)
        } else {
            (None, None)
        };

        let (logging_layer, stdout_guard) = init_logging::<Registry>(&config.log)?;
        let registry = tracing_subscriber::registry()
            .with(logging_layer)
            .with(otel_layer);

        registry.init();

        let context = TelemetryContext::from_propagation_config_with_meter(
            &config.telemetry.tracing.propagation,
            &config.log,
            metrics_provider
                .as_ref()
                .map(|provider| provider.meter_with_scope(scope)),
        );

        let prometheus = create_prometheus_runtime(config, prometheus_config.as_ref())?;

        Ok(Self {
            traces_provider: tracer_provider,
            metrics_provider,
            prometheus,
            context,
            logging_writer_guard: stdout_guard,
        })
    }

    /// Initializes telemetry for cases where the subscriber should not be set globally.
    /// Used only for tests because of the global static MAX_LEVEL in tracing, which makes it
    /// impossible to have concurrent telemetry-enabled and telemetry-disabled tests without
    /// affecting each other.
    #[cfg(feature = "testing")]
    pub fn init_testing_subscriber(
        config: &HiveRouterConfig,
    ) -> Result<(Self, impl tracing::Subscriber), TelemetryInitError> {
        let resource = build_resource(&config.telemetry)?;
        let scope = build_scope();
        let otel_layer_result = build_otel_layer_from_config(
            &config.telemetry,
            RandomIdGenerator::default(),
            scope.clone(),
            resource.clone(),
        )?;
        let metrics_result = build_meter_provider_from_config(&config.telemetry, resource)?;

        let (otel_layer, tracer_provider) = match otel_layer_result {
            Some((layer, provider)) => {
                set_tracing_enabled(true);
                (Some(layer), Some(provider))
            }
            None => {
                // skip disabling tracing when config has no telemetry enabled.
                // set_tracing_enabled() writes to a global static atomic (MAX_LEVEL). when
                // runnin no-telemetry tests, it will disable span creation process-wide and
                // break any concurrent yes-telemetry tests that expect traces.
                //
                // yeah this is hacky but it's necessary because of the MAX_LEVEL global static,
                // if we were to make MAX_LEVEL thread-local, it would hurt performance and
                // the only place we need MAX_LEVEL to be thread-local is in tests...
                //
                // instead, we simply leave the MAX_LEVEL untouched, even when telemetry is disabled.
                // this has no impact because when telemetry is disabled, the otel_layer will be None
                // and wont create or export any traces/spans
                //
                // set_tracing_enabled(false);
                (None, None)
            }
        };

        let meter = metrics_result
            .as_ref()
            .map(|setup| setup.provider.meter_with_scope(scope));
        let context = TelemetryContext::from_propagation_config_with_meter(
            &config.telemetry.tracing.propagation,
            &config.log,
            meter,
        );

        let (logging_layer, logging_writer_guard) = init_logging::<Registry>(&config.log)?;

        let subscriber = tracing_subscriber::Registry::default()
            .with(logging_layer)
            .with(otel_layer);

        let prometheus_config = metrics_result
            .as_ref()
            .and_then(|setup| setup.prometheus.as_ref());
        let prometheus = create_prometheus_runtime(config, prometheus_config)?;

        Ok((
            Self {
                traces_provider: tracer_provider,
                metrics_provider: metrics_result.map(|setup| setup.provider),
                prometheus,
                context,
                logging_writer_guard,
            },
            subscriber,
        ))
    }

    pub async fn graceful_shutdown(&self) {
        use tokio::task::spawn_blocking;

        let tracer = self.traces_provider.clone();
        let meter_provider = self.metrics_provider.clone();
        let shutdown_tracer = spawn_blocking(|| {
            if let Some(provider) = tracer {
                tracing::debug!(
                    target: targets::TELEMETRY,
                    layer = "provider",
                    "shutdown scheduled"
                );

                let _ = provider.force_flush();
                let _ = provider.shutdown();

                tracing::info!(
                    target: targets::TELEMETRY,
                    layer = "provider",
                    "shutdown completed"
                );
            }
        });

        let shutdown_prometheus = async {
            if let Some(runtime) = &self.prometheus {
                tracing::debug!(
                    target: targets::TELEMETRY,
                    layer = "prometheus",
                    "shutdown scheduled"
                );
                runtime.shutdown().await;
                tracing::info!(
                    target: targets::TELEMETRY,
                    layer = "prometheus",
                    "shutdown completed"
                );
            }
        };

        let shutdown_metrics = spawn_blocking(|| {
            if let Some(provider) = meter_provider {
                tracing::debug!(
                    target: targets::TELEMETRY,
                    layer = "metrics",
                    "shutdown scheduled"
                );
                let _ = provider.force_flush();
                let _ = provider.shutdown();
                tracing::info!(
                    target: targets::TELEMETRY,
                    layer = "metrics",
                    "shutdown completed"
                );
            }
        });

        let _ = tokio::join!(shutdown_tracer, shutdown_metrics, shutdown_prometheus);
    }
}

fn create_prometheus_runtime(
    config: &HiveRouterConfig,
    prometheus_config: Option<&PrometheusRuntimeConfig>,
) -> Result<Option<PrometheusRuntime>, TelemetryInitError> {
    let Some(prometheus_config) = prometheus_config else {
        return Ok(None);
    };

    let registry = prometheus_config.registry.clone();
    let router_port = config.port();
    let port = prometheus_config.port.unwrap_or(router_port);
    let same_listener = router_port == port;

    if same_listener {
        return Ok(Some(PrometheusRuntime::Attached(PrometheusAttached {
            registry: registry.clone(),
            endpoint: normalize_route_path(&prometheus_config.path),
        })));
    }

    let path = prometheus_config.path.clone();
    let path_for_log = prometheus_config.path.clone();

    let registry_for_result = registry.clone();
    let path_for_result = path.clone();

    let listen_address = (config.host(), port);
    let server = HttpServer::new(move || {
        let registry = registry.clone();
        let path = path.clone();
        async move {
            App::new()
                .state(registry)
                .service(web::resource(path).route(web::get().to(metrics_handler)))
                .default_service(web::to(|| async { HttpResponse::NotFound() }))
        }
    })
    .workers(1)
    .disable_signals()
    .bind(listen_address);

    let server = {
        let server = server?;
        server.run()
    };

    tracing::info!(
        target: targets::TELEMETRY,
        layer = "metrics",
        port = %port,
        path = %path_for_log,
        "Prometheus metrics server started"
    );

    let server_for_result = server.clone();
    let handle = tokio::spawn(async move {
        if let Err(err) = server.await {
            tracing::error!(
                target: targets::TELEMETRY,
                layer = "metrics",
                error = ?err,
                "Prometheus metrics server failed"
            );
        }
    });

    Ok(Some(PrometheusRuntime::Detached {
        registry: registry_for_result,
        endpoint: path_for_result,
        server: server_for_result,
        handle: Mutex::new(handle),
    }))
}

pub(crate) fn build_metrics_response(registry: &prometheus::Registry) -> HttpResponse {
    let encoder = TextEncoder::new();
    let metric_families = registry.gather();
    let mut buffer = Vec::new();

    if let Err(err) = encoder.encode(&metric_families, &mut buffer) {
        tracing::error!(
            target: targets::TELEMETRY,
            layer = "metrics",
            error = ?err,
            "failed to encode metrics"
        );

        return HttpResponse::InternalServerError()
            .body(format!("failed to encode metrics: {err}"));
    }

    HttpResponse::Ok()
        .content_type(encoder.format_type())
        .body(buffer)
}

async fn metrics_handler(registry: web::types::State<prometheus::Registry>) -> HttpResponse {
    build_metrics_response(&registry)
}

pub fn init_logging<S>(config: &LoggingConfig) -> Result<(DynLayer<S>, WorkerGuard), ParseError>
where
    S: tracing::Subscriber
        + for<'span> tracing_subscriber::registry::LookupSpan<'span>
        + Send
        + Sync,
{
    let stdout_stream = std::io::stdout();
    let is_terminal = stdout_stream.is_terminal();
    let (stdout_writer, stdout_guard) =
        tracing_appender::non_blocking::NonBlockingBuilder::default()
            .lossy(false)
            .finish(stdout_stream);
    let targets_filter = create_targets_filter(&config.level, config.log_internals);
    let ignore_otel_filter = create_ignore_otel_filter();
    let env_filter =
        EnvFilter::from_str(config.env_filter_str())?.add_directive(config.level.as_str().parse()?);
    let stdout_layer = tracing_subscriber::fmt::layer();
    let timer = UtcTime::rfc_3339();

    let layer = match config.format {
        LogFormat::Json => stdout_layer
            .event_format(RouterJsonFormat)
            .with_writer(stdout_writer)
            .with_filter(targets_filter)
            .with_filter(env_filter)
            .with_filter(ignore_otel_filter)
            .boxed(),
        LogFormat::Text => {
            let compact = tracing_subscriber::fmt::format::Format::default()
                .compact()
                .with_thread_ids(false)
                .with_timer(timer)
                .with_target(true)
                .with_ansi(is_terminal);

            stdout_layer
                .event_format(RouterTextFormat(compact))
                .with_writer(stdout_writer)
                .with_filter(targets_filter)
                .with_filter(env_filter)
                .with_filter(ignore_otel_filter)
                .boxed()
        }
    };

    Ok((layer, stdout_guard))
}

/// Context for telemetry operations that doesn't rely on global state.
#[derive(Clone)]
pub struct TelemetryContext {
    propagator: Option<Arc<TextMapCompositePropagator>>,
    pub metrics: Arc<Metrics>,
    meter: Option<Meter>,
    pub logging_correlation_extractor: RequestIdentifierExtractor,
}

impl TelemetryContext {
    /// Creates a telemetry context from tracing propagation config
    pub fn from_propagation_config(
        telemetry_config: &TracingPropagationConfig,
        log_config: &LoggingConfig,
    ) -> Self {
        Self::from_propagation_config_with_meter(telemetry_config, log_config, None)
    }

    pub fn from_propagation_config_with_meter(
        telemetry_config: &TracingPropagationConfig,
        log_config: &LoggingConfig,
        meter: Option<Meter>,
    ) -> Self {
        #[allow(deprecated)]
        use opentelemetry_jaeger_propagator::Propagator as JaegerPropagator;
        use opentelemetry_sdk::propagation::{BaggagePropagator, TraceContextPropagator};
        #[allow(deprecated)]
        use opentelemetry_zipkin::Propagator as B3Propagator;

        let mut propagators: Vec<Box<dyn TextMapPropagator + Send + Sync>> = Vec::new();

        if telemetry_config.trace_context {
            propagators.push(Box::new(TraceContextPropagator::new()));
        }

        if telemetry_config.baggage {
            propagators.push(Box::new(BaggagePropagator::new()));
        }

        if telemetry_config.b3 {
            warn!(target = targets::TELEMETRY, "Zipkin exporter is deprecated. Use the OTLP exporter instead. Refer to https://zipkin.io/pages/architecture.html for Zipkin's native OTLP support.");
            #[allow(deprecated)]
            propagators.push(Box::new(B3Propagator::new()));
        }

        if telemetry_config.jaeger {
            warn!(target = targets::TELEMETRY, "Jaeger propagation format is deprecated. Use W3C TraceContext propagation instead. See https://www.jaegertracing.io/sdk-migration/#propagation-format");
            #[allow(deprecated)]
            propagators.push(Box::new(JaegerPropagator::new()));
        }

        let metrics = Arc::new(Metrics::new(meter.as_ref()));

        let logging_correlation_extractor =
            RequestIdentifierExtractor::new(log_config.correlation.clone());

        if propagators.is_empty() {
            return Self {
                propagator: None,
                metrics,
                meter,
                logging_correlation_extractor,
            };
        }

        Self {
            propagator: Some(Arc::new(TextMapCompositePropagator::new(propagators))),
            metrics,
            meter,
            logging_correlation_extractor,
        }
    }

    pub fn inject_context<I>(&self, injector: &mut I)
    where
        I: Injector,
    {
        use tracing_opentelemetry::OpenTelemetrySpanExt;

        if let Some(propagator) = &self.propagator {
            let current_context = tracing::Span::current().context();
            propagator.inject_context(&current_context, injector);
        }
    }

    pub fn inject_context_into_http_headers(&self, headers: &mut http::HeaderMap) {
        self.inject_context(&mut HeaderMapInjector::from(headers));
    }

    pub fn extract_context<E>(&self, extractor: &E) -> opentelemetry::Context
    where
        E: opentelemetry::propagation::Extractor,
    {
        if let Some(propagator) = &self.propagator {
            propagator.extract(extractor)
        } else {
            opentelemetry::Context::new()
        }
    }

    /// Returns true if this context has a propagator configured
    pub fn is_enabled(&self) -> bool {
        self.propagator.is_some()
    }

    pub fn meter(&self) -> Option<&Meter> {
        self.meter.as_ref()
    }
}

pub fn build_otel_layer_from_config<S, I>(
    config: &TelemetryConfig,
    id_generator: I,
    scope: InstrumentationScope,
    resource: Resource,
) -> Result<Option<(impl Layer<S> + Send + Sync + 'static, SdkTracerProvider)>, TelemetryError>
where
    S: Subscriber + for<'span> LookupSpan<'span> + Send + Sync + 'static,
    I: IdGenerator + 'static,
{
    if !config.is_tracing_enabled() {
        return Ok(None);
    }

    let traces_provider = build_trace_provider(config, id_generator, resource.clone())?;
    let tracer = traces_provider.tracer_with_scope(scope);
    let traces_layer = tracing_opentelemetry::layer()
        .with_tracer(tracer)
        .with_tracked_inactivity(false)
        .with_location(false)
        .with_threads(false)
        // Drop events from tracing macros (info!, error!, etc.),
        // but accept those from span.add_event()
        .with_filter(filter_fn(|metadata| {
            metadata.is_span() && *metadata.level() <= tracing::Level::INFO
        }));

    Ok(Some((traces_layer, traces_provider)))
}

pub fn build_scope() -> InstrumentationScope {
    InstrumentationScope::builder("graphql-hive.router")
        .with_version(env!("CARGO_PKG_VERSION"))
        .build()
}

pub fn build_resource(config: &TelemetryConfig) -> Result<Resource, TelemetryError> {
    let resolved_attributes =
        resolve_string_map(&config.resource.attributes, "resource attribute")?;

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

    Ok(Resource::builder()
        .with_attributes(resource_attributes)
        .build())
}
