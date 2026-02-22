use hive_router_config::log::shared::LogLevel;
use tracing_subscriber::{EnvFilter, Layer};

static INTERNAL_CRATES: &[&str] = &[
    "ntex_server",
    "ntex_rt",
    "ntex_service",
    "hyper_rustls",
    "ntex_net",
    "ntex_io",
    "ntex",
    "hyper_util",
];

pub fn create_env_filter(log_level: &LogLevel, internals: bool) -> EnvFilter {
    let mut filters: Vec<String> = Vec::new();

    filters.push(log_level.as_str().to_string());

    for crate_name in INTERNAL_CRATES {
        if internals {
            filters.push(format!("{}={}", crate_name, log_level.as_str()));
        } else {
            filters.push(format!("{}=off", crate_name));
        }
    }

    EnvFilter::builder()
        .parse(filters.join(","))
        .expect("Failed to parse logging filters")
}

pub type DynLayer<S> = Box<dyn Layer<S> + Send + Sync + 'static>;
