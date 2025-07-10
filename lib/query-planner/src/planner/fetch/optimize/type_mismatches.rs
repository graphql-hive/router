use petgraph::graph::NodeIndex;
use tracing::{instrument, trace};

use crate::{
    ast::{
        merge_path::{MergePath, Segment},
        mismatch_finder::SelectionMismatchFinder,
        safe_merge::SafeSelectionSetMerger,
        selection_item::SelectionItem,
        type_aware_selection::{field_condition_equal, find_selection_set_by_path_mut},
    },
    planner::{
        fetch::{error::FetchGraphError, fetch_graph::FetchGraph},
        plan_nodes::{FetchNodePathSegment, FetchRewrite, KeyRenamer},
    },
    state::supergraph_state::SupergraphState,
};

impl FetchGraph {
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
            let mismatches_paths =
                finder.find_mismatches_in_node(&node.service_name, &node.output_new);

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
                let mut merger = SafeSelectionSetMerger::default();

                if let Some(Segment::Field(field_lookup, args_hash_lookup, condition)) =
                    mismatch_path.last()
                {
                    // TODO: We can avoid this cut and slice thing, if we return "SelectionItem" instead of "SelectionSet" inside "find_selection_set_by_path_mut".
                    let lookup_path = &mismatch_path.without_last();
                    let root_def_selections =
                        node.output_new.selections_for_definition(&root_def_name);

                    if let Some(selection_set) =
                        find_selection_set_by_path_mut(root_def_selections, lookup_path)
                    {
                        let next_alias = merger.safe_next_alias_name(&selection_set.items);
                        let item = selection_set
                          .items
                          .iter_mut()
                          .find(|v| matches!(v, SelectionItem::Field(field) if field.name == *field_lookup && field.arguments_hash() == *args_hash_lookup && field_condition_equal(condition, field)));

                        if let Some(SelectionItem::Field(field_to_alias)) = item {
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
                                    rename_key_to: field_to_alias.name.to_string(),
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
