use graphql_parser::query as query_ast;
use graphql_parser::schema as schema_ast;

pub struct NormalizationContext<'a> {
    pub operation_name: Option<&'a str>,
    pub document: &'a mut query_ast::Document<'static, String>,
    pub schema: &'a schema_ast::Document<'static, String>,
}
