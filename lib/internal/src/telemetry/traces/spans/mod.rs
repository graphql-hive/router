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
pub mod graphql;
pub mod http_request;
pub mod kind;

#[cfg(test)]
mod tests;
