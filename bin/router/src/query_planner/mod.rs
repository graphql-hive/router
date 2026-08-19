// TODO: enable this cross-lib to avoid panics
// #![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

pub mod ast;
pub mod consumer_schema;
pub mod federation_spec;
pub mod graph;
pub mod utils;

pub mod planner;
pub mod state;

#[cfg(test)]
mod tests;
