use directives::JoinFieldDirective;
use graphql_parser::{
    parse_query,
    query::{Definition, OperationDefinition, SelectionSet},
};

use crate::state::supergraph_state::{SupergraphDefinition, SupergraphField};

pub(crate) mod definitions;
pub(crate) mod directives;

pub(crate) mod directive_trait;
pub(crate) mod inacessible;
pub(crate) mod join_enum_value;
pub(crate) mod join_field;
pub(crate) mod join_graph;
pub(crate) mod join_implements;
pub(crate) mod join_type;
pub(crate) mod join_union;

pub struct FederationRules;

impl FederationRules {
    pub fn parse_provides(
        join_field: &JoinFieldDirective,
    ) -> Option<SelectionSet<'static, String>> {
        if let Some(provides) = &join_field.provides {
            let selection_set_str = format!("{{{provides}}}");
            // TODO: Far from ideal, but we can use the graphql_parser here to get it parsed for us
            let parsed_doc = parse_query(&selection_set_str).unwrap().into_static();
            let parsed_definition = parsed_doc
                .definitions
                .first()
                .expect("failed to parse selection set for provides")
                .clone();

            let maybe_selection_set = match parsed_definition {
                Definition::Operation(OperationDefinition::SelectionSet(selection_set)) => {
                    Some(selection_set)
                }
                _ => return None,
            };

            return maybe_selection_set;
        }

        None
    }

    pub fn check_field_subgraph_availability<'a>(
        field: &'a SupergraphField,
        current_subgraph: &str,
        parent_definition: &SupergraphDefinition,
    ) -> (bool, Option<&'a JoinFieldDirective>) {
        let involved_subgraphs = parent_definition.subgraphs();

        // A field i available if: it has no @join__field directives at all
        if field.join_field.is_empty() {
            // AND its parent type is available in the subgraph
            if involved_subgraphs.contains(&current_subgraph) {
                return (true, None);
            }

            // No join_field and not available in parent
            return (false, None);
        }

        // Find the relevant join_field and use it to determine availability
        let join_field = field.join_field.iter().find(|join_field| {
            join_field
                .graph_id
                .as_ref()
                .is_some_and(|g| g == current_subgraph)
        });

        if let Some(join_field) = join_field {
            return (true, Some(join_field));
        }

        (false, None)
    }
}
