mod consumer_schema;
mod satisfiability_graph;
mod utils;

pub mod operation_advisor;
pub mod supergraph_metadata;

pub fn parse_schema(sdl: &str) -> graphql_parser_hive_fork::schema::Document<'static, String> {
    graphql_parser_hive_fork::parse_schema(sdl)
        .unwrap()
        .into_static()
}
