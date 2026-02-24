use std::{io::IsTerminal, str::FromStr};

use hive_router_config::{log::LogFormat, HiveRouterConfig};
use hive_router_internal::telemetry::{
    build_otel_layer_from_config,
    error::TelemetryError,
    otel::{
        opentelemetry::{global::set_tracer_provider, propagation::Extractor},
        opentelemetry_sdk::trace::{RandomIdGenerator, SdkTracerProvider},
    },
    traces::set_tracing_enabled,
    TelemetryContext,
};
use tracing_subscriber::{filter::filter_fn, util::SubscriberInitExt};
use tracing_subscriber::{fmt::time::UtcTime, EnvFilter, Layer};
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

pub struct Telemetry {
    pub provider: Option<SdkTracerProvider>,
    pub context: TelemetryContext,
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

        let registry = tracing_subscriber::registry().with(otel_layer);
        init_logging(config, registry)?;

        let context =
            TelemetryContext::from_propagation_config(&config.telemetry.tracing.propagation);

        Ok(Self {
            provider: tracer_provider,
            context,
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
        use tracing_subscriber::Registry;

        let otel_layer_result =
            build_otel_layer_from_config(&config.telemetry, RandomIdGenerator::default())?;

        let (otel_layer, tracer_provider) = match otel_layer_result {
            Some((layer, provider)) => {
                set_tracing_enabled(true);
                (Some(layer), Some(provider))
            }
            None => {
                // skip calling disabling tracing when config has no telemetry enabled.
                // set_tracing_enabled() writes to a global static atomic (MAX_LEVEL). when
                // runnin no-telemetry tests, it will disable span creation process-wide and
                // break any concurrent yes-telemetry tests that expect traces.
                //
                // yeah this is hacky but it's necessary because of the MAX_LEVEL global static,
                // if we were to make MAX_LEVEL thread-local, it would hurt performance and
                // the only place we need MAX_LEVEL to be thread-local is in tests...
                //
                // set_tracing_enabled(false);
                (None, None)
            }
        };

        let context =
            TelemetryContext::from_propagation_config(&config.telemetry.tracing.propagation);

        let filter = EnvFilter::from_str(config.log.env_filter_str())?;

        let subscriber = Registry::default()
            .with(filter)
            .with(otel_layer)
            .with(fmt::layer().with_test_writer());

        Ok((
            Self {
                provider: tracer_provider,
                context,
            },
            subscriber,
        ))
    }

    pub async fn graceful_shutdown(&self) {
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
