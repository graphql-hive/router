use graphql_parser::query::{Definition, Document, OperationDefinition};

// TODO: improve
pub fn get_operation_to_execute<'a>(
    document: &'a Document<'static, String>,
    // operation_name: Option<&str>,
) -> Option<&'a OperationDefinition<'static, String>> {
    document
        .definitions
        .iter()
        .find_map(|definition| match definition {
            Definition::Operation(operation) => Some(operation),
            _ => None,
        })
}
