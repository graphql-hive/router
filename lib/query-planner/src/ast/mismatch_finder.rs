use std::collections::{HashMap, VecDeque};

use tracing::{instrument, trace};

use crate::{
    ast::{
        merge_path::{MergePath, Segment},
        selection_item::SelectionItem,
        selection_set::{FieldSelection, SelectionSet},
    },
    planner::fetch::{selections::FetchStepSelections, state::MultiTypeFetchStep},
    state::{
        subgraph_state::{SubgraphDefinition, SubgraphState},
        supergraph_state::{SubgraphName, SupergraphState, TypeNode},
    },
};

#[derive(Debug)]
pub struct SelectionMismatchFinder<'a> {
    supergraph_state: &'a SupergraphState,
}

type MismatchesFound = Vec<(String, MergePath)>;

impl<'a> SelectionMismatchFinder<'a> {
    pub fn new(supergraph_state: &'a SupergraphState) -> Self {
        Self { supergraph_state }
    }

    #[instrument(level = "trace", skip_all, fields(subgraph_name,))]
    pub fn find_mismatches_in_node(
        &self,
        subgraph_name: &SubgraphName,
        selections: &FetchStepSelections<MultiTypeFetchStep>,
    ) -> MismatchesFound {
        let mut mismtaches_found = MismatchesFound::new();
        let subgraph_state = self
            .supergraph_state
            .subgraphs_state
            .get(subgraph_name)
            .unwrap();

        for (definition_name, selection_set) in selections.iter_selections() {
            let entrypoint_type = subgraph_state.definitions.get(definition_name).unwrap();
            let start_path = MergePath::default();

            handle_selection_set(
                definition_name,
                self.supergraph_state,
                subgraph_state,
                entrypoint_type,
                selection_set,
                start_path,
                &mut mismtaches_found,
            );

            trace!("found total of {} mismatches", mismtaches_found.len());
        }

        mismtaches_found
    }
}

/// Handles a selection set by traversing all selection in a specific level.
/// In case of a field: checks for mismatches and collects information.
/// In case of a fragment spread: "flattens" the selections and updates the path.
#[instrument(level = "trace", skip_all, fields(
  parent_def = parent_def.name(),
  selection = format!("{}", selection_set)
))]
fn handle_selection_set<'field, 'schema>(
    root_def_type_name: &str,
    supergraph_state: &'schema SupergraphState,
    subgraph_state: &'schema SubgraphState,
    parent_def: &'schema SubgraphDefinition,
    selection_set: &'field SelectionSet,
    parent_path: MergePath,
    mismatches_found: &mut MismatchesFound,
) {
    // A HashMap of "field_name" to "field_return_type". When we encounter a field in a given selection-set level,
    // we update the HashMap with the field's name and return type.
    // We use this to monitor and find mismatches later.
    let mut encountered_field_to_type: HashMap<&'field str, &'schema TypeNode> = HashMap::new();

    // A queue for traversing the selection set. In case of an inline fragment, we "flatten" the selections and update the path.
    // Values are: (ParentType, Path, Selections)
    let mut traversal_queue =
        VecDeque::from([(parent_def, parent_path, selection_set.items.iter())]);

    while let Some((type_def, path, selections_group)) = traversal_queue.pop_front() {
        for selection_item in selections_group {
            match selection_item {
                SelectionItem::Field(field) => {
                    if field.is_introspection_field() {
                        continue;
                    }

                    let next_path = path.push(Segment::Field(
                        field.name.clone(),
                        field.arguments_hash(),
                        field.into(),
                    ));

                    let next_parent_type_name = handle_field(
                        root_def_type_name,
                        supergraph_state,
                        type_def,
                        field,
                        &next_path,
                        &mut encountered_field_to_type,
                        mismatches_found,
                    );

                    if let Some(next_parent_def) =
                        next_parent_type_name.and_then(|n| subgraph_state.definitions.get(n))
                    {
                        handle_selection_set(
                            root_def_type_name,
                            supergraph_state,
                            subgraph_state,
                            next_parent_def,
                            &field.selections,
                            next_path,
                            mismatches_found,
                        );
                    }
                }
                SelectionItem::FragmentSpread(_) => {
                    unreachable!("fragment spread is not expected at this stage")
                }
                SelectionItem::InlineFragment(fragment) => {
                    let fragment_type = subgraph_state
                        .definitions
                        .get(&fragment.type_condition)
                        .unwrap();
                    let fragment_enter_path = path.push(Segment::Cast(
                        fragment.type_condition.clone(),
                        fragment.into(),
                    ));

                    traversal_queue.push_back((
                        fragment_type,
                        fragment_enter_path,
                        fragment.selections.items.iter(),
                    ));
                }
            }
        }
    }
}

/// Handles a field by finding it's subgraph-specific type (or the default type if none is set),
/// and then checks if we've already encountered this field in the current selection set level.
/// In case the field encountered, we check if the field is compatible or not (have a mismatch).
///
/// Returns the return type of the selection, if the inner selection needs to be processed (in case nested selections are defined).
fn handle_field<'field, 'schema>(
    root_def_type_name: &str,
    state: &'schema SupergraphState,
    parent_def: &'schema SubgraphDefinition,
    field: &'field FieldSelection,
    field_path: &MergePath,
    encountered_field_to_type: &mut HashMap<&'field str, &'schema TypeNode>,
    mismatches_found: &mut MismatchesFound,
) -> Option<&'schema str> {
    let parent_def_fields = parent_def.fields().unwrap();
    let field_name = field.name.as_str();
    let field_type = parent_def_fields
        .iter()
        .find_map(|f| {
            if f.name == field_name {
                Some(
                    f.join_field
                        .as_ref()
                        .and_then(|jf| jf.type_in_graph.as_ref())
                        .unwrap_or(&f.field_type),
                )
            } else {
                None
            }
        })
        .unwrap();

    if let Some(maybe_conflicting_type) = encountered_field_to_type.get(field_name) {
        if !maybe_conflicting_type.can_be_merged_with(field_type) {
            let left_is_composite = state
                .definitions
                .get(maybe_conflicting_type.inner_type())
                .is_some_and(|v| v.is_composite_type());
            let right_is_composite = state
                .definitions
                .get(field_type.inner_type())
                .is_some_and(|v| v.is_composite_type());

            if !left_is_composite || !right_is_composite {
                trace!(
                  "found a conflicting type for a selection field '{}', conflict is: '{}' <-> '{}', path: {}",
                  field_name,
                  maybe_conflicting_type,
                  field_type,
                  field_path,
              );

                mismatches_found.push((root_def_type_name.to_string(), field_path.clone()));
            }
        }
    } else {
        encountered_field_to_type.insert(field_name, field_type);
    }

    if field.is_leaf() {
        None
    } else {
        Some(field_type.inner_type())
    }
}
