// TODO: enable this cross-lib to avoid panics
// #![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

pub mod ast;
pub mod consumer_schema;
mod federation_spec;
pub mod graph;
pub mod utils;

pub mod planner;
pub mod state;

#[cfg(test)]
mod tests;

pub fn parse_schema(sdl: &str) -> graphql_parser::schema::Document<'static, String> {
    graphql_parser::parse_schema(sdl).unwrap().into_static()
}

pub fn parse_operation(operation: &str) -> graphql_parser::query::Document<'static, String> {
    graphql_parser::parse_query(operation)
        .unwrap()
        .into_static()
}
