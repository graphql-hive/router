use directives::JoinFieldDirective;

use crate::supergraph_metadata::{SupergraphDefinition, SupergraphField};

pub mod definitions;
pub mod directives;

pub(crate) mod inacessible;
pub(crate) mod join_field;
pub(crate) mod join_implements;
pub(crate) mod join_type;

pub struct FederationRules;

impl FederationRules {
    pub fn is_external_field(join_field: &JoinFieldDirective) -> bool {
        join_field.external.is_some_and(|v| v) && join_field.requires.is_none()
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
                .graph
                .as_ref()
                .is_some_and(|g| g == current_subgraph)
        });

        if let Some(join_field) = join_field {
            return (true, Some(join_field));
        }

        return (false, None);
    }

    pub fn is_field_accessible(field: &SupergraphField) -> bool {
        !field.inaccessible
    }
}
