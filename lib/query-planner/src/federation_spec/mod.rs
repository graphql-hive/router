use directives::JoinFieldDirective;
use graphql_tools::parser::{
    parse_query,
    query::{Definition, OperationDefinition, Selection, SelectionSet},
};

use crate::{
    ast::{
        normalization::{context::RootTypes, normalize_operation_mut},
        selection_item::SelectionItem,
        selection_set::{FieldSelection, InlineFragmentSelection},
        type_aware_selection::TypeAwareSelection,
    },
    state::supergraph_state::{SupergraphDefinition, SupergraphField, SupergraphState},
};

pub(crate) mod definitions;
pub(crate) mod directives;

pub mod authorization;
pub mod demand_control;
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
    subgraph_name: &str,
    fields_str: &str,
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

fn parse_key_fields_argument_value(
    supergraph: &SupergraphState,
    type_name: &str,
    key: &str,
) -> crate::ast::selection_set::SelectionSet {
    let selection_set_str = format!("{{{key}}}");
    let parsed = parse_query::<String>(&selection_set_str)
        .unwrap_or_else(|err| panic!("Key parse error for {type_name}: {err}"));

    let Some(Definition::Operation(OperationDefinition::SelectionSet(selection_set))) =
        parsed.definitions.into_iter().next()
    else {
        unreachable!("key fields string '{{...}}' did not produce a SelectionSet")
    };

    resolve_key_selection_set(supergraph, type_name, &selection_set)
}

fn resolve_key_selection_set(
    supergraph: &SupergraphState,
    current_type_name: &str,
    selection_set: &SelectionSet<'_, String>,
) -> crate::ast::selection_set::SelectionSet {
    let mut items = selection_set
        .items
        .iter()
        .filter_map(|selection| match selection {
            Selection::Field(field) => Some(resolve_key_field(supergraph, current_type_name, field)),
            Selection::InlineFragment(fragment) => Some(resolve_key_fragment(supergraph, fragment)),
            Selection::FragmentSpread(_) => {
                panic!("Fragment spread is not supported in federation key fields")
            }
        })
        .collect::<Vec<_>>();
    items.sort();

    crate::ast::selection_set::SelectionSet { items }
}

fn resolve_key_field(
    supergraph: &SupergraphState,
    current_type_name: &str,
    field: &graphql_tools::parser::query::Field<'_, String>,
) -> SelectionItem {
    if field.name == "__typename" {
        return SelectionItem::Field(FieldSelection::new_typename());
    }

    assert!(field.arguments.is_empty(), "Arguments are not supported in federation key fields");
    assert!(field.directives.is_empty(), "Directives are not supported in federation key fields");

    let current_type = supergraph
        .definitions
        .get(current_type_name)
        .unwrap_or_else(|| panic!("Type '{current_type_name}' is not defined in supergraph"));

    let supergraph_field = current_type.fields().get(&field.name).unwrap_or_else(|| {
        panic!("Type '{current_type_name}' does not have field '{}'", field.name)
    });

    let output_type_name = supergraph_field.field_type.inner_type();
    let selections = if field.selection_set.items.is_empty() {
        crate::ast::selection_set::SelectionSet::default()
    } else {
        resolve_key_selection_set(supergraph, output_type_name, &field.selection_set)
    };

    SelectionItem::Field(FieldSelection {
        name: field.name.clone(),
        selections,
        alias: field.alias.clone(),
        arguments: None,
        skip_if: None,
        include_if: None,
        omit_from_response: false,
    })
}

fn resolve_key_fragment(
    supergraph: &SupergraphState,
    fragment: &graphql_tools::parser::query::InlineFragment<'_, String>,
) -> SelectionItem {
    let type_condition = fragment
        .type_condition
        .as_ref()
        .map(crate::ast::normalization::utils::extract_type_condition)
        .unwrap_or_else(|| panic!("Inline fragment without type condition is not supported in federation key fields"));

    let type_definition = supergraph
        .definitions
        .get(type_condition)
        .unwrap_or_else(|| panic!("Type '{type_condition}' is not defined in supergraph"));

    SelectionItem::InlineFragment(InlineFragmentSelection {
        type_condition: type_definition.name().to_string(),
        selections: resolve_key_selection_set(supergraph, type_definition.name(), &fragment.selection_set),
        skip_if: None,
        include_if: None,
    })
}

/// Walks a `sizedFields` selection set top-down, enforcing exactly one field per
/// level and rejecting fragments, appending each field name to `path`.
fn collect_sized_field_path(
    selection_set: &SelectionSet<'_, String>,
    original: &str,
    path: &mut Vec<String>,
) -> Result<(), String> {
    if selection_set.items.is_empty() {
        return Ok(());
    }

    if selection_set.items.len() != 1 {
        return Err(format!(
            "'sizedFields' entry '{original}' must select exactly one field per level, found {}",
            selection_set.items.len()
        ));
    }

    match &selection_set.items[0] {
        Selection::Field(field) => {
            path.push(field.name.clone());
            collect_sized_field_path(&field.selection_set, original, path)
        }
        Selection::FragmentSpread(_) | Selection::InlineFragment(_) => Err(format!(
            "'sizedFields' entry '{original}' must only contain fields, not fragments"
        )),
    }
}

pub struct FederationRules;

impl FederationRules {
    pub fn parse_key<'a>(
        supergraph: &'a SupergraphState,
        subgraph_name: &str,
        type_name: &'a str,
        key: &str,
    ) -> TypeAwareSelection<'a> {
        // TODO: This intentionally bypasses normalize_fields_argument_value_mut() because the
        // old normalization-heavy path made Graph::graph_from_supergraph_state() spend tens of
        // seconds in parse_key() during the first graph build after SupergraphState construction.
        // If we switch back to the old path for semantic consistency (type expansion, fragment
        // spread expansion, merge/dedup behavior), we need a solution that preserves first-build
        // performance, not just a per-build cache inside graph construction.
        let _ = subgraph_name;
        let selection_set = parse_key_fields_argument_value(supergraph, type_name, key);
        TypeAwareSelection {
            type_name,
            selection_set,
        }
    }

    pub fn parse_provides<'a>(
        supergraph: &'a SupergraphState,
        join_field: &JoinFieldDirective,
        subgraph_name: &str,
        type_name: &'a str,
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

    pub fn parse_requires<'a>(
        supergraph: &'a SupergraphState,
        subgraph_name: &str,
        type_name: &'a str,
        requires: &str,
    ) -> SelectionSet<'static, String> {
        normalize_fields_argument_value_mut(supergraph, type_name, subgraph_name, requires)
    }

    /// Parses a single `@listSize(sizedFields: [...])` entry, written in
    /// selection-set form, into the flat field path it points at:
    ///
    /// - `"field"` → `["field"]`
    /// - `"field { nested }"` → `["field", "nested"]`
    pub fn parse_list_size_sized_fields(
        sized_fields_selection: &str,
    ) -> Result<Vec<String>, String> {
        let selection_set_str = format!("{{{sized_fields_selection}}}");
        let parsed = parse_query::<String>(&selection_set_str).map_err(|err| {
            format!("invalid 'sizedFields' entry '{sized_fields_selection}': {err}")
        })?;

        let Some(Definition::Operation(OperationDefinition::SelectionSet(selection_set))) =
            parsed.definitions.into_iter().next()
        else {
            return Err(format!(
                "'sizedFields' entry '{sized_fields_selection}' is not a valid field selection"
            ));
        };

        let mut path = Vec::new();
        collect_sized_field_path(&selection_set, sized_fields_selection, &mut path)?;
        Ok(path)
    }

    pub fn check_field_subgraph_availability<'a>(
        field: &'a SupergraphField,
        current_subgraph_id: &str,
        parent_definition: &SupergraphDefinition,
    ) -> (bool, Option<&'a JoinFieldDirective>) {
        // A field i available if: it has no @join__field directives at all
        if field.join_field.is_empty() {
            // AND its parent type is available in the subgraph
            if parent_definition
                .join_types()
                .iter()
                .any(|join_type| join_type.graph_id == current_subgraph_id)
            {
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
