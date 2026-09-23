//! Spans are created through small wrapper structs (see `graphql` and `http_request`) rather
//! than ad-hoc `tracing` calls.
//!
//! The wrappers enforce consistent naming, attributes, and sampling gates,
//! and provide focused helpers for recording common fields and events.
//!
//! Attribute keys live in `attributes` as `const` values to avoid typos, keep keys consistent
//! across crates, and make refactors safer.
//! Those attributes are also tested for correctness in `tests`.
//!
//! Each span/event includes `hive.kind`, which tags the semantic role of the span or event.
//! `HiveSpanKind` enumerates supported span kinds (e.g. `graphql.operation`, `http.server`),
//! while `HiveEventKind` enumerates event kinds (e.g. GraphQL error events).
pub const TARGET_NAME: &str = "hive-router";

pub mod attributes;
pub mod coprocessor;
pub mod graphql;
pub mod http_request;
pub mod kind;
pub mod observed_error;

/// Converts a number into the `i64` a span field needs to be exported as an OpenTelemetry int.
///
/// `tracing-opentelemetry` maps `i64` fields to int attributes, but has no `record_u64`, so
/// unsigned values (`u16`, `u64`, `usize`, ...) fall back to `record_debug` and are exported
/// as strings. Record every numeric field through this helper. Values above `i64::MAX`
/// saturate instead of wrapping.
pub fn otel_int(value: impl TryInto<i64>) -> i64 {
    value.try_into().unwrap_or(i64::MAX)
}

#[cfg(test)]
pub(crate) mod tests;
