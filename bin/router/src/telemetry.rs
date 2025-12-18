use std::{io::IsTerminal, str::FromStr};

use hive_router_config::{
    log::LogFormat, telemetry::tracing::TracingPropagationConfig, HiveRouterConfig,
};
use hive_router_internal::telemetry::{
    otel::{
        opentelemetry::{
            global::{set_text_map_propagator, set_tracer_provider},
            propagation::{Extractor, TextMapCompositePropagator, TextMapPropagator},
        },
        opentelemetry_jaeger_propagator::Propagator as JaegerPropagator,
        opentelemetry_sdk::{
            logs::SdkLoggerProvider,
            propagation::{BaggagePropagator, TraceContextPropagator},
            trace::{RandomIdGenerator, SdkTracerProvider},
        },
        opentelemetry_zipkin::Propagator as B3Propagator,
    },
    traces::set_tracing_enabled,
    OpenTelemetry,
};
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt::time::UtcTime, EnvFilter};
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

#[derive(Default, Clone)]
pub(crate) struct OpenTelemetryProviders {
    pub logger: Option<SdkLoggerProvider>,
    pub tracer: Option<SdkTracerProvider>,
}

impl OpenTelemetryProviders {
    pub(crate) async fn graceful_shutdown(&self) {
        use tokio::task::spawn_blocking;

        let tracer = self.tracer.clone();
        let shutdown_tracer = spawn_blocking(|| {
            if let Some(provider) = tracer {
                let _ = provider.shutdown();
            }
        });

        let logger_provider = self.logger.clone();
        let shutdown_logger = spawn_blocking(|| {
            if let Some(provider) = logger_provider {
                let _ = provider.shutdown();
            }
        });

        let _ = tokio::join!(shutdown_tracer, shutdown_logger);
    }
}

pub(crate) fn init(config: &HiveRouterConfig) -> OpenTelemetryProviders {
    let timer = UtcTime::rfc_3339();
    let filter = EnvFilter::from_str(config.log.env_filter_str())
        .unwrap_or_else(|e| panic!("failed to initialize env-filter logger: {}", e));

    let id_generator = RandomIdGenerator::default();

    init_propagators(&config.telemetry.tracing.propagation);

    let OpenTelemetry { tracer, logger } =
        OpenTelemetry::from_config(&config.telemetry, id_generator).unwrap();

    if let Some(tracer) = &tracer {
        set_tracing_enabled(true);
        set_tracer_provider(tracer.provider.clone());
    } else {
        set_tracing_enabled(false);
    }

    let tracer_provider = tracer.as_ref().map(|t| t.provider.clone());
    let logger_provider = logger.as_ref().map(|l| l.provider.clone());

    let registry = tracing_subscriber::registry()
        .with(tracer.map(|t| t.layer))
        .with(logger.map(|l| l.layer));

    let is_terminal = std::io::stdout().is_terminal();
    match config.log.format {
        LogFormat::PrettyTree => registry
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
                    .with_targets(false),
            )
            .with(filter)
            .init(),
        LogFormat::Json => registry
            .with(fmt::layer().json().with_timer(timer))
            .with(filter)
            .init(),
        LogFormat::PrettyCompact => registry
            .with(
                fmt::layer()
                    .compact()
                    .with_ansi(is_terminal)
                    .with_timer(timer),
            )
            .with(filter)
            .init(),
    };

    OpenTelemetryProviders {
        tracer: tracer_provider,
        logger: logger_provider,
    }
}

fn init_propagators(config: &TracingPropagationConfig) {
    let mut propagators: Vec<Box<dyn TextMapPropagator + Send + Sync>> = Vec::new();

    if config.trace_context {
        propagators.push(Box::new(TraceContextPropagator::new()));
    }

    if config.baggage {
        propagators.push(Box::new(BaggagePropagator::new()))
    }

    if config.b3 {
        propagators.push(Box::new(B3Propagator::new()));
    }

    if config.jaeger {
        propagators.push(Box::new(JaegerPropagator::new()));
    }

    if propagators.is_empty() {
        return;
    }

    set_text_map_propagator(TextMapCompositePropagator::new(propagators));
}
