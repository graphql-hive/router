use petgraph::visit::{Bfs, IntoNodeReferences};
use tracing::{instrument, trace};

use crate::{
    ast::{
        merge_path::{MergePath, Segment},
        selection_item::SelectionItem,
        type_aware_selection::find_selection_set_by_path_mut,
    },
    planner::fetch::{error::FetchGraphError, fetch_graph::FetchGraph},
};

impl FetchGraph {
    /// This method applies internal aliasing for fields in the fetch graph.
    /// In case a fetch step contains a record of alias made to an output field, it needs to be propagated to all descendants steps that depends on this
    /// output field, in multiple locations:
    /// 1. In "input" selections
    /// 2. In "response_path"
    #[instrument(level = "trace", skip_all)]
    pub(crate) fn apply_internal_aliases_patching(&mut self) -> Result<(), FetchGraphError> {
        // First, iterate and find all nodes that needed to perform internal aliasing for fields
        let mut nodes_with_aliases = self
            .graph
            .node_references()
            .filter_map(|(index, node)| {
                if !node.internal_aliases_locations.is_empty() {
                    Some((index, node.internal_aliases_locations.clone()))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();

        trace!(
            "found total of {} node with internal aliased fields",
            nodes_with_aliases.len(),
        );

        while let Some((aliased_node_index, scoped_aliases_locations)) = nodes_with_aliases.pop() {
            for (root_type_name, aliases_locations) in scoped_aliases_locations {
                let mut bfs = Bfs::new(&self.graph, aliased_node_index);

                trace!(
                    "Iterating step [{}], total of {} aliased fields in output selections of type {}",
                    aliased_node_index.index(),
                    aliases_locations.len(),
                    root_type_name
                );

                // Iterate and find all possible children of a node that needed aliasing.
                // We can't really tell which nodes are affected, as they might be at any level of the hierarchy, so we travel the graph.
                while let Some(decendent_idx) = bfs.next(&self.graph) {
                    if decendent_idx != aliased_node_index {
                        let decendent = self.get_step_data_mut(decendent_idx)?;

                        trace!(
                            "Checking if decendent [{}] is relevant for aliasing patching...",
                            decendent_idx.index()
                        );

                        for (alias_path, new_name) in aliases_locations.iter() {
                            // Last segment is the field that was aliased
                            let maybe_patched_field = alias_path.last();
                            // Build a path without the alias path, to make sure we don't patch the wrong field
                            let relative_path =
                                decendent.response_path.slice_from(alias_path.len());

                            if let Some(Segment::Field(field_name, args_hash, condition)) =
                                maybe_patched_field
                            {
                                trace!(
                              "field '{}' was aliased, relative selection path: '{}', checking if need to patch selection '{}'",
                              field_name,
                              relative_path,
                              decendent.input.selection_set
                          );

                                // First, check if the node's input selection set contains the field that was aliased
                                if let Some(selection) = find_selection_set_by_path_mut(
                                    &mut decendent.input.selection_set,
                                    &relative_path,
                                ) {
                                    trace!("found selection to patch: {}", selection);
                                    let item_to_patch = selection.items.iter_mut().find(|item| matches!(item, SelectionItem::Field(field) if field.name == *field_name && field.arguments_hash() == *args_hash));

                                    if let Some(SelectionItem::Field(field_to_patch)) =
                                        item_to_patch
                                    {
                                        field_to_patch.alias = Some(field_to_patch.name.clone());
                                        field_to_patch.name = new_name.clone();

                                        trace!(
                                      "path '{}' found in selection, patched applied, new selection: {}",
                                      relative_path,
                                      field_to_patch
                                  );
                                    }
                                } else {
                                    trace!(
                                        "path '{}' was not found in selection '{}', skipping...",
                                        relative_path,
                                        decendent.input.selection_set
                                    );
                                }

                                // Then, check if the node's response_path is using the part that was aliased
                                let segment_idx_to_patch = decendent
                              .response_path
                              .inner
                              .iter()
                              .enumerate()
                              .find_map(|(idx, part)| {
                                  if matches!(part, Segment::Field(f, a, c) if f == field_name && a == args_hash && c == condition) {
                                      Some(idx)
                                  } else {
                                      None
                                  }
                              });

                                if let Some(segment_idx_to_patch) = segment_idx_to_patch {
                                    trace!(
                                "Node [{}] is using aliased field {} in response_path (segment idx: {}, alias: {:?})",
                                decendent_idx.index(),
                                field_name,
                                segment_idx_to_patch,
                                alias_path
                            );

                                    let mut new_path = (*decendent.response_path.inner).to_vec();

                                    if let Some(Segment::Field(name, _, _)) =
                                        new_path.get_mut(segment_idx_to_patch)
                                    {
                                        *name = new_name.clone();
                                        decendent.response_path = MergePath::new(new_path);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }
}
