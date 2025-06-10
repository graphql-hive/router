use graphql_parser::query as query_ast;
use graphql_parser::schema as schema_ast;

use crate::consumer_schema::schema_metadata::PossibleTypesMap;

pub struct NormalizationContext<'a> {
    pub operation_name: Option<&'a str>,
    pub document: &'a mut query_ast::Document<'static, String>,
    pub schema: &'a schema_ast::Document<'static, String>,
    pub possible_types: &'a PossibleTypesMap,
}
