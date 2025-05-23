pub fn parse_schema(sdl: &str) -> graphql_parser::schema::Document<'static, String> {
    graphql_parser::parse_schema(sdl).unwrap().into_static()
}

pub fn parse_operation(operation: &str) -> graphql_parser::query::Document<'static, String> {
    graphql_parser::parse_query(operation)
        .unwrap()
        .into_static()
}
