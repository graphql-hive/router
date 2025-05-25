pub(crate) mod arguments;
pub mod document;
pub(crate) mod merge_path;
pub mod operation;
pub mod selection_item;
pub(crate) mod selection_set;
pub(crate) mod type_aware_selection;
pub(crate) mod value;

use graphql_parser::{
    parse_query,
    query::{Definition, OperationDefinition},
};

pub fn parse_selection_set(
    selection_set_str: &str,
) -> graphql_parser::query::SelectionSet<'static, String> {
    let parsed_doc = parse_query(selection_set_str).unwrap().into_static();
    let parsed_definition = parsed_doc
        .definitions
        .first()
        .expect("failed to parse selection set")
        .clone();

    match parsed_definition {
        Definition::Operation(OperationDefinition::SelectionSet(selection_set)) => {
            Some(selection_set)
        }
        _ => None,
    }
    .expect("invalid selection set")
}
