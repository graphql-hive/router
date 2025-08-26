#[inline]
pub fn parse_schema(sdl: &str) -> graphql_parser::schema::Document<'static, String> {
    graphql_parser::parse_schema(sdl).unwrap().into_static()
}

#[inline]
pub fn parse_operation(operation: &str) -> graphql_parser::query::Document<'static, String> {
    graphql_parser::parse_query(operation)
        .unwrap()
        .into_static()
}

#[inline]
pub fn safe_parse_operation(
    operation: &str,
) -> Result<graphql_parser::query::Document<'static, String>, graphql_parser::query::ParseError> {
    graphql_parser::parse_query(operation).map(|op| op.into_static())
}
