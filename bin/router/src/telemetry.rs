use std::{io::IsTerminal, str::FromStr};

use hive_router_config::{log::LogFormat, HiveRouterConfig};
use hive_router_internal::telemetry::{
    build_otel_layer_from_config,
    otel::{
        opentelemetry::{global::set_tracer_provider, propagation::Extractor},
        opentelemetry_sdk::trace::{RandomIdGenerator, SdkTracerProvider},
    },
    traces::set_tracing_enabled,
};
use tracing_subscriber::util::SubscriberInitExt;
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

#[derive(Default, Clone)]
pub struct OpenTelemetryProviders {
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

        let _ = tokio::join!(shutdown_tracer);
    }
}

pub fn init_logging(config: &HiveRouterConfig, registry: Registry) {
    let timer = UtcTime::rfc_3339();
    let filter = EnvFilter::from_str(config.log.env_filter_str())
        .unwrap_or_else(|e| panic!("failed to initialize env-filter logger: {}", e));
    let is_terminal = std::io::stdout().is_terminal();
    match config.log.format {
        LogFormat::PrettyTree => {
            let _ = registry
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
                .try_init();
        }
        LogFormat::Json => {
            let _ = registry
                .with(fmt::layer().json().with_timer(timer))
                .with(filter)
                .try_init();
        }
        LogFormat::PrettyCompact => {
            let _ = registry
                .with(
                    fmt::layer()
                        .compact()
                        .with_ansi(is_terminal)
                        .with_timer(timer),
                )
                .with(filter)
                .try_init();
        }
    };
}

pub fn init(config: &HiveRouterConfig) -> OpenTelemetryProviders {
    let id_generator = RandomIdGenerator::default();

    let otel_layer_result = build_otel_layer_from_config(&config.telemetry, id_generator).unwrap();

    let tracer_provider = if let Some((layer, provider)) = otel_layer_result {
        set_tracing_enabled(true);
        set_tracer_provider(provider.clone());
        // Layer already includes filter for dropping non-span events
        let _registry = tracing_subscriber::registry().with(layer);

        Some(provider)
    } else {
        set_tracing_enabled(false);
        None
    };

    init_logging(config, tracing_subscriber::registry());

    OpenTelemetryProviders {
        tracer: tracer_provider,
    }
}
