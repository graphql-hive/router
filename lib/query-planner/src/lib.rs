mod consumer_schema;
mod federation_spec;
mod graph;
mod utils;

pub mod operation_advisor;
pub mod state;

pub fn parse_schema(sdl: &str) -> graphql_parser_hive_fork::schema::Document<'static, String> {
    graphql_parser_hive_fork::parse_schema(sdl)
        .unwrap()
        .into_static()
}

pub fn parse_operation(
    operation: &str,
) -> graphql_parser_hive_fork::query::Document<'static, String> {
    graphql_parser_hive_fork::parse_query(operation)
        .unwrap()
        .into_static()
}
