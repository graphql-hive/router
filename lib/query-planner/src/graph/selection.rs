use graphql_parser_hive_fork::query::SelectionSet;

pub struct GraphSelectionSet {}

impl GraphSelectionSet {
    pub fn new(selection_set: SelectionSet<'static, String>) -> Self {
        GraphSelectionSet {}
    }
}
