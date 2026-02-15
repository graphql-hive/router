use hive_router_config::HiveRouterConfig;
use hive_router_internal::{
    logging::logging_layers_from_logger_config,
    telemetry::{
        build_otel_layer_from_config,
        error::TelemetryError,
        otel::{
            opentelemetry::{global::set_tracer_provider, propagation::Extractor},
            opentelemetry_sdk::trace::{RandomIdGenerator, SdkTracerProvider},
        },
        traces::set_tracing_enabled,
        TelemetryContext,
    },
};
use tracing::debug;
use tracing_subscriber::{
    fmt::{self},
    layer::SubscriberExt,
};
use tracing_subscriber::{util::SubscriberInitExt, Registry};

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

pub struct Telemetry {
    pub provider: Option<SdkTracerProvider>,
    pub context: TelemetryContext,
    pub writer_guards: Vec<tracing_appender::non_blocking::WorkerGuard>,
}

#[derive(Debug, thiserror::Error)]
pub enum TelemetryInitError {
    #[error(transparent)]
    TelemetryError(#[from] TelemetryError),
    #[error("failed to initialize env-filter logger: {0}")]
    EnvFilter(#[from] tracing_subscriber::filter::ParseError),
}

impl Telemetry {
    /// Sets up the global tracing subscriber including logging and OpenTelemetry.
    pub fn init_global(config: &HiveRouterConfig) -> Result<Self, TelemetryInitError> {
        let id_generator = RandomIdGenerator::default();
        let otel_layer_result = build_otel_layer_from_config(&config.telemetry, id_generator)?;

        let (otel_layer, tracer_provider) = if let Some((layer, provider)) = otel_layer_result {
            set_tracing_enabled(true);
            set_tracer_provider(provider.clone());
            (Some(layer), Some(provider))
        } else {
            set_tracing_enabled(false);
            (None, None)
        };

        let (logging_layers, writer_guards) =
            logging_layers_from_logger_config::<Registry>(&config.log);

        let registry = tracing_subscriber::registry()
            .with(logging_layers)
            .with(otel_layer);
        registry.init();

        let context =
            TelemetryContext::from_propagation_config(&config.telemetry.tracing.propagation);

        Ok(Self {
            provider: tracer_provider,
            context,
            writer_guards,
        })
    }

    /// Initializes telemetry for cases where the subscriber should not be set globally
    pub fn init_subscriber(
        config: &HiveRouterConfig,
    ) -> Result<(Self, impl tracing::Subscriber), TelemetryInitError> {
        let otel_layer_result =
            build_otel_layer_from_config(&config.telemetry, RandomIdGenerator::default())?;

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

        let context =
            TelemetryContext::from_propagation_config(&config.telemetry.tracing.propagation);

        // let filter = EnvFilter::from_str(config.log.env_filter_str())?;

        let subscriber = Registry::default()
            // .with(filter)
            .with(otel_layer)
            .with(fmt::layer().with_test_writer());

        Ok((
            Self {
                provider: tracer_provider,
                context,
                writer_guards: vec![],
            },
            subscriber,
        ))
    }

    pub async fn graceful_shutdown(&self) {
        debug!("flushing telemetry data");
        use tokio::task::spawn_blocking;

        let tracer = self.provider.clone();
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

        let _ = tokio::join!(shutdown_tracer);
    }
}
