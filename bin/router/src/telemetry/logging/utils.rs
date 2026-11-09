use crate::config::log::LogLevel;
use tracing::level_filters::LevelFilter;
use tracing_subscriber::{
    filter::{filter_fn, Targets},
    Layer,
};

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

pub fn create_ignore_otel_filter(
) -> tracing_subscriber::filter::FilterFn<impl Fn(&tracing::Metadata<'_>) -> bool> {
    // the hint keeps a filter combined with this one from losing its own (`And` takes the
    // stricter of two hints, and none when either has none)
    filter_fn(|metadata| {
        if metadata.is_span() && metadata.fields().field("hive.kind").is_some() {
            return false;
        }

        true
    })
    .with_max_level_hint(LevelFilter::TRACE)
}

pub fn create_targets_filter(log_level: &LogLevel, internals: bool) -> Targets {
    let level_filter: LevelFilter = log_level.into();

    Targets::new()
        .with_targets(INTERNAL_CRATES.iter().map(|crate_name| {
            (
                *crate_name,
                match internals {
                    true => level_filter,
                    false => LevelFilter::OFF,
                },
            )
        }))
        .with_default(level_filter)
}

pub type DynLayer<S> = Box<dyn Layer<S> + Send + Sync + 'static>;
