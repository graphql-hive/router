use std::{io::IsTerminal, str::FromStr, sync::Mutex};

use hive_router_config::{log::LogFormat, HiveRouterConfig};
use hive_router_internal::{
    http::normalize_route_path,
    telemetry::{
        build_otel_layer_from_config, build_resource, build_scope,
        error::TelemetryError,
        metrics::{build_meter_provider_from_config, PrometheusRuntimeConfig},
        otel::{
            opentelemetry::{
                global::{set_meter_provider, set_tracer_provider},
                metrics::MeterProvider,
                propagation::Extractor,
            },
            opentelemetry_sdk::{
                metrics::SdkMeterProvider,
                trace::{RandomIdGenerator, SdkTracerProvider},
            },
        },
        traces::set_tracing_enabled,
        TelemetryContext,
    },
};
use ntex::web::{self};
use ntex::web::{App, HttpResponse, HttpServer};
use prometheus::{Encoder, TextEncoder};
use tracing_subscriber::{filter::filter_fn, util::SubscriberInitExt, Layer};
use tracing_subscriber::{fmt::time::UtcTime, EnvFilter, Registry};
use tracing_subscriber::{
    fmt::{self},
    layer::SubscriberExt,
};

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

        let registry = tracing_subscriber::registry().with(otel_layer);
        init_logging(config, registry)?;

        let context = TelemetryContext::from_propagation_config_with_meter(
            &config.telemetry.tracing.propagation,
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
        })
    }

    /// Initializes telemetry for cases where the subscriber should not be set globally
    pub fn init_subscriber(
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
                set_tracing_enabled(false);
                (None, None)
            }
        };

        let meter = metrics_result
            .as_ref()
            .map(|setup| setup.provider.meter_with_scope(scope));
        let context = TelemetryContext::from_propagation_config_with_meter(
            &config.telemetry.tracing.propagation,
            meter,
        );

        let filter = EnvFilter::from_str(config.log.env_filter_str())?;

        let subscriber = Registry::default()
            .with(filter)
            .with(otel_layer)
            .with(fmt::layer().with_test_writer());

        Ok((
            Self {
                traces_provider: tracer_provider,
                metrics_provider: metrics_result.map(|setup| setup.provider),
                prometheus: None,
                context,
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
                tracing::info!(
                    component = "telemetry",
                    layer = "provider",
                    "shutdown scheduled"
                );
                let _ = provider.force_flush();
                let _ = provider.shutdown();
                tracing::info!(
                    component = "telemetry",
                    layer = "provider",
                    "shutdown completed"
                );
            }
        });

        let shutdown_prometheus = async {
            if let Some(runtime) = &self.prometheus {
                tracing::info!(
                    component = "telemetry",
                    layer = "prometheus",
                    "shutdown scheduled"
                );
                runtime.shutdown().await;
                tracing::info!(
                    component = "telemetry",
                    layer = "prometheus",
                    "shutdown completed"
                );
            }
        };

        let shutdown_metrics = spawn_blocking(|| {
            if let Some(provider) = meter_provider {
                tracing::info!(
                    component = "telemetry",
                    layer = "metrics",
                    "shutdown scheduled"
                );
                let _ = provider.force_flush();
                let _ = provider.shutdown();
                tracing::info!(
                    component = "telemetry",
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
    let router_port = config.http.port();
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

    let listen_address = (config.http.host(), port);
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

    let server = match server {
        Ok(server) => server.run(),
        Err(err) => return Err(err.into()),
    };

    tracing::info!(
        component = "telemetry",
        layer = "metrics",
        port = %port,
        path = %path_for_log,
        "Prometheus metrics server started"
    );

    let server_for_result = server.clone();
    let handle = tokio::spawn(async move {
        if let Err(err) = server.await {
            tracing::error!(
                component = "telemetry",
                layer = "metrics",
                error = %err,
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

pub fn init_logging<S>(config: &HiveRouterConfig, registry: S) -> Result<(), TelemetryInitError>
where
    S: tracing::Subscriber
        + for<'span> tracing_subscriber::registry::LookupSpan<'span>
        + Send
        + Sync,
{
    let timer = UtcTime::rfc_3339();
    let filter = EnvFilter::from_str(config.log.env_filter_str())?;
    let is_terminal = std::io::stdout().is_terminal();

    let events_only = filter_fn(|m| !m.is_span());

    match config.log.format {
        LogFormat::PrettyTree => {
            registry
                .with(
                    tracing_tree::HierarchicalLayer::new(2)
                        .with_ansi(is_terminal)
                        .with_bracketed_fields(true)
                        .with_deferred_spans(false)
                        .with_wraparound(25)
                        .with_indent_lines(true)
                        .with_timer(tracing_tree::time::Uptime::default())
                        .with_thread_names(false)
                        .with_thread_ids(false)
                        .with_targets(false)
                        .with_filter(events_only),
                )
                .with(filter)
                .init();
        }
        LogFormat::Json => {
            registry
                .with(
                    fmt::layer()
                        .json()
                        .with_timer(timer)
                        .with_filter(events_only),
                )
                .with(filter)
                .init();
        }
        LogFormat::PrettyCompact => {
            registry
                .with(
                    fmt::layer()
                        .compact()
                        .with_ansi(is_terminal)
                        .with_timer(timer)
                        .with_filter(events_only),
                )
                .with(filter)
                .init();
        }
    };

    Ok(())
}
