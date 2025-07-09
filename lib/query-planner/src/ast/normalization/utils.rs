use graphql_parser::query as query_ast;

pub fn extract_type_condition<'a, 'd, T: query_ast::Text<'d>>(
    type_condition: &'a query_ast::TypeCondition<'d, T>,
) -> &'a str {
    match type_condition {
        query_ast::TypeCondition::On(v) => v.as_ref(),
    }
}
