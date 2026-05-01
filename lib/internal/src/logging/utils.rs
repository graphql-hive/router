use hive_router_config::log::shared::LogLevel;
use tracing::level_filters::LevelFilter;
use tracing_subscriber::{filter::Targets, Layer};

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

pub fn create_targets_filter(log_level: &LogLevel, internals: bool) -> Targets {
    let level_filter: LevelFilter = log_level.into();

    Targets::new()
        .with_targets(
            INTERNAL_CRATES
                .iter()
                .map(|crate_name| {
                    (
                        *crate_name,
                        match internals {
                            true => level_filter,
                            false => LevelFilter::OFF,
                        },
                    )
                })
                .collect::<Vec<(&str, LevelFilter)>>(),
        )
        .with_default(level_filter)
}

pub type DynLayer<S> = Box<dyn Layer<S> + Send + Sync + 'static>;
