use std::collections::HashSet;

use petgraph::graph::NodeIndex;
use tracing::{instrument, trace};

use crate::query_planner::{
    ast::{
        merge_path::{MergePath, Segment},
        mismatch_finder::SelectionMismatchFinder,
        selection_item::SelectionItem,
        selection_set::{
            field_condition_equal, fields_through_fragments, find_selection_set_by_path,
            find_selection_set_by_path_mut, SelectionSet,
        },
    },
    planner::{
        fetch::{error::FetchGraphError, fetch_graph::FetchGraph, state::MultiTypeFetchStep},
        plan_nodes::{FetchNodePathSegment, FetchRewrite, KeyRenamer},
    },
    state::supergraph_state::SupergraphState,
};

impl FetchGraph<MultiTypeFetchStep> {
    #[instrument(level = "trace", skip_all)]
    pub(crate) fn fix_conflicting_type_mismatches(
        &mut self,
        supergraph: &SupergraphState,
    ) -> Result<(), FetchGraphError> {
        let mut pending_patches = Vec::<(NodeIndex, Vec<(String, MergePath)>)>::new();

        for (node_index, node) in self.all_nodes() {
            if self.root_index.is_some_and(|v| v == node_index) {
                continue;
            }

            trace!(
                "looking for type conflict mismatches in node [{}]",
                node_index.index()
            );

            let finder = SelectionMismatchFinder::new(supergraph);
            let mismatches_paths = finder.find_mismatches_in_node(&node.service_name, &node.output);

            if !mismatches_paths.is_empty() {
                pending_patches.push((node_index, mismatches_paths));
            }
        }

        let mut pending_output_rewrites = Vec::<(NodeIndex, FetchRewrite)>::new();

        for (node_index, mismatches_paths) in pending_patches {
            let node = self.get_step_data_mut(node_index)?;

            trace!(
                "fixing {} mismatch conflicts in node [{}] by using aliases",
                mismatches_paths.len(),
                node_index.index()
            );

            for (root_def_name, mismatch_path) in mismatches_paths {
                if let Some(Segment::Field(field_seg, args_hash_lookup, condition)) =
                    mismatch_path.last()
                {
                    // TODO: We can avoid this cut and slice thing, if we return "SelectionItem" instead of "SelectionSet" inside "find_selection_set_by_path_mut".
                    let lookup_path = &mismatch_path.without_last();
                    let root_def_selections = node
                        .output
                        .selections_for_definition_mut(&root_def_name)
                        .expect("missing definition in step");

                    let next_alias = free_alias(root_def_selections, lookup_path);
                    if let Some(selection_set) =
                        find_selection_set_by_path_mut(root_def_selections, lookup_path)
                    {
                        let item = selection_set
                          .items
                          .iter_mut()
                          .find(|v| matches!(v, SelectionItem::Field(field) if field.selection_identifier() == field_seg.response_key() && field.arguments_hash() == *args_hash_lookup && field_condition_equal(condition, field)));

                        if let Some(SelectionItem::Field(field_to_alias)) = item {
                            let original_response_key =
                                field_to_alias.selection_identifier().to_string();

                            trace!(
                                "applying alias '{}' to existing field '{}' at path '{}'",
                                next_alias,
                                field_to_alias.name,
                                lookup_path
                            );

                            let mut output_rewrite_path: Vec<FetchNodePathSegment> =
                                lookup_path.into();
                            output_rewrite_path.push(FetchNodePathSegment::Key(next_alias.clone()));

                            pending_output_rewrites.push((
                                node_index,
                                FetchRewrite::KeyRenamer(KeyRenamer {
                                    rename_key_to: original_response_key,
                                    path: output_rewrite_path,
                                }),
                            ));

                            field_to_alias.alias = Some(next_alias);
                        }
                    }
                }
            }
        }

        for (node_index, output_rewrite) in pending_output_rewrites {
            let node = self.get_step_data_mut(node_index)?;

            trace!(
                "adding output rewrite to node [{}]: {:?}",
                node_index.index(),
                output_rewrite
            );

            node.add_output_rewrite(output_rewrite);
        }

        Ok(())
    }
}

/// An alias no field of the object at `path` uses yet. The object's keys are all of its fields,
/// in every fragment, not just the ones next to the field: `TypeB` and `TypeC` fragments on one
/// object can't both get `_internal_qp_alias_0`, and neither can a field next to a client's
/// `_internal_qp_alias_0: __typename`.
fn free_alias(selections: &SelectionSet, path: &MergePath) -> String {
    let mut object_path = path.clone();
    while matches!(object_path.last(), Some(Segment::TypeCondition(..))) {
        object_path = object_path.without_last();
    }
    let taken: HashSet<&str> = find_selection_set_by_path(selections, &object_path)
        .map(|object| {
            fields_through_fragments(object)
                .into_iter()
                .map(|field| field.selection_identifier())
                .collect()
        })
        .unwrap_or_default();

    (0..)
        .map(|index| format!("_internal_qp_alias_{index}"))
        .find(|alias| !taken.contains(alias.as_str()))
        .unwrap()
}
