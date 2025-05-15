use graphql_parser_hive_fork::{
    parse_query,
    query::{Definition, OperationDefinition},
};

pub mod selection_item;
pub mod selection_set;
pub mod type_aware_selection;

pub fn parse_selection_set(
    selection_set_str: &str,
) -> graphql_parser_hive_fork::query::SelectionSet<'static, String> {
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
