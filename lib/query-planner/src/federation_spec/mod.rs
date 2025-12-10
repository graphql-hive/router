use directives::JoinFieldDirective;
use graphql_parser::{
    parse_query,
    query::{Definition, OperationDefinition, SelectionSet},
};

use crate::{
    ast::{
        normalization::{context::RootTypes, normalize_operation_mut},
        type_aware_selection::TypeAwareSelection,
    },
    state::supergraph_state::{SupergraphDefinition, SupergraphField, SupergraphState},
};

pub(crate) mod definitions;
pub(crate) mod directives;

pub mod authorization;
pub(crate) mod directive_trait;
pub(crate) mod inacessible;
pub(crate) mod join_directive;
pub(crate) mod join_enum_value;
pub(crate) mod join_field;
pub(crate) mod join_graph;
pub(crate) mod join_implements;
pub(crate) mod join_owner;
pub(crate) mod join_type;
pub(crate) mod join_union;

fn normalize_fields_argument_value_mut(
    supergraph: &SupergraphState,
    type_name: &str,
    subgraph_name: &String,
    fields_str: &String,
) -> SelectionSet<'static, String> {
    let selection_set_str = format!("{{{fields_str}}}");
    // TODO: Far from ideal, but we can use the graphql_parser here to get it parsed for us
    let mut parsed_doc = parse_query(&selection_set_str).unwrap().into_static();

    normalize_operation_mut(
        supergraph,
        &mut parsed_doc,
        None,
        Some(RootTypes {
            query: Some(type_name),
            mutation: None,
            subscription: None,
        }),
        Some(subgraph_name),
    )
    .unwrap_or_else(|err| panic!("Normalization error: {err}"));

    match parsed_doc
        .definitions
        .first()
        .expect("failed to parse selection set")
    {
        Definition::Operation(OperationDefinition::SelectionSet(selection_set)) => {
            selection_set.to_owned()
        }
        _ => {
            unreachable!(
                "Internal error: 'fields' string '{{...}}' did not result in a SelectionSet"
            )
        }
    }
}

pub struct FederationRules;

impl FederationRules {
    pub fn parse_key(
        supergraph: &SupergraphState,
        subgraph_name: &String,
        type_name: &str,
        key: &String,
    ) -> TypeAwareSelection {
        let selection_set =
            normalize_fields_argument_value_mut(supergraph, type_name, subgraph_name, key);
        TypeAwareSelection {
            type_name: type_name.to_string(),
            selection_set: selection_set.into(),
        }
    }

    pub fn parse_provides(
        supergraph: &SupergraphState,
        join_field: &JoinFieldDirective,
        subgraph_name: &String,
        type_name: &str,
    ) -> Option<SelectionSet<'static, String>> {
        if let Some(provides) = &join_field.provides {
            return Some(normalize_fields_argument_value_mut(
                supergraph,
                type_name,
                subgraph_name,
                provides,
            ));
        }

        None
    }

    pub fn parse_requires(
        supergraph: &SupergraphState,
        subgraph_name: &String,
        type_name: &str,
        requires: &String,
    ) -> SelectionSet<'static, String> {
        normalize_fields_argument_value_mut(supergraph, type_name, subgraph_name, requires)
    }

    pub fn check_field_subgraph_availability<'a>(
        field: &'a SupergraphField,
        current_subgraph_id: &str,
        parent_definition: &SupergraphDefinition,
    ) -> (bool, Option<&'a JoinFieldDirective>) {
        let involved_subgraphs = parent_definition.subgraphs();

        // A field i available if: it has no @join__field directives at all
        if field.join_field.is_empty() {
            // AND its parent type is available in the subgraph
            if involved_subgraphs.contains(&current_subgraph_id) {
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
                .is_some_and(|g| g == current_subgraph_id)
        });

        if let Some(join_field) = join_field {
            return (true, Some(join_field));
        }

        (false, None)
    }
}
