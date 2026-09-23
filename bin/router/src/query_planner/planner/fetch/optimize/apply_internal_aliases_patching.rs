use petgraph::visit::{Bfs, IntoNodeReferences};
use tracing::{instrument, trace};

use crate::query_planner::{
    ast::{
        merge_path::{MergePath, Segment},
        selection_item::SelectionItem,
        selection_set::find_selection_set_by_path_mut,
    },
    planner::fetch::{error::FetchGraphError, fetch_graph::FetchGraph, state::MultiTypeFetchStep},
};

impl FetchGraph<MultiTypeFetchStep> {
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
            let aliased_node_path = self
                .get_step_data(aliased_node_index)?
                .response_path
                .clone();

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

                            if let Some(Segment::Field(field_seg, args_hash, condition)) =
                                maybe_patched_field
                            {
                                // Where the object holding the aliased field sits in the response.
                                // `alias_path` starts at the aliased step's root.
                                let parent_path =
                                    aliased_node_path.concat(&alias_path.without_last());

                                // TODO: Avoid "except" here of course.
                                let decendent_type_name = decendent
                                    .input
                                    .try_as_single()
                                    .ok_or_else(|| {
                                      FetchGraphError::Internal(
                                        format!(
                                          "Expected single input type for descendant node [{}] during alias patching, but found multi-type input",
                                          decendent_idx.index()
                                        )
                                      )
                                    })?
                                    .to_string();

                                let selection = decendent
                                    .input
                                    .selections_for_definition_mut(&decendent_type_name)
                                    .expect("selection set is missing");

                                trace!(
                              "field '{}' was aliased under '{}', checking if need to patch selection '{}'",
                              field_seg.field_name(),
                              parent_path,
                              selection
                          );

                                // First, check if the node's input selection set contains the field that was aliased.
                                // That's only possible when the node reads objects at or above that parent.
                                let relative_path =
                                    path_below(&parent_path, &decendent.response_path);
                                if let Some(selection) =
                                    relative_path.as_ref().and_then(|relative_path| {
                                        find_selection_set_by_path_mut(selection, relative_path)
                                    })
                                {
                                    trace!("found selection to patch: {}", selection);
                                    let item_to_patch = selection.items.iter_mut().find(|item| matches!(item, SelectionItem::Field(field) if field.name == field_seg.field_name() && field.arguments_hash() == *args_hash));

                                    if let Some(SelectionItem::Field(field_to_patch)) =
                                        item_to_patch
                                    {
                                        field_to_patch.alias = Some(field_to_patch.name.clone());
                                        field_to_patch.name = new_name.clone();

                                        trace!(
                                      "path '{}' found in selection, patched applied, new selection: {}",
                                      parent_path,
                                      field_to_patch
                                  );
                                    }
                                } else {
                                    trace!(
                                        "path '{}' is not read by decendent [{}], skipping...",
                                        parent_path,
                                        decendent_idx.index()
                                    );
                                }

                                // Then, check if the node's response_path is using the part that was aliased
                                let segment_idx_to_patch = decendent
                              .response_path
                              .inner
                              .iter()
                              .enumerate()
                              .find_map(|(idx, part)| {
                                  if matches!(part, Segment::Field(ref f, a, c) if f.field_name() == field_seg.field_name() && a == args_hash && c == condition) {
                                      Some(idx)
                                  } else {
                                      None
                                  }
                              });

                                if let Some(segment_idx_to_patch) = segment_idx_to_patch {
                                    trace!(
                                "Node [{}] is using aliased field {} in response_path (segment idx: {}, alias: {:?})",
                                decendent_idx.index(),
                                field_seg.field_name(),
                                segment_idx_to_patch,
                                alias_path
                            );

                                    let mut new_path = (*decendent.response_path.inner).to_vec();

                                    if let Some(Segment::Field(ref mut seg, _, _)) =
                                        new_path.get_mut(segment_idx_to_patch)
                                    {
                                        seg.field_name = new_name.clone();
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

/// If `prefix` points at `path` or at one of its parents, returns the rest of `path` below it.
///
/// Type conditions are skipped. Fields match by response key and arguments.
fn path_below(path: &MergePath, prefix: &MergePath) -> Option<MergePath> {
    let path = path.without_type_castings();
    let prefix = prefix.without_type_castings();

    if prefix.len() > path.len() {
        return None;
    }

    let same = |a: &Segment, b: &Segment| match (a, b) {
        (Segment::List, Segment::List) => true,
        (Segment::Field(a, a_args, _), Segment::Field(b, b_args, _)) => {
            a.response_key() == b.response_key() && a_args == b_args
        }
        _ => false,
    };

    if prefix
        .inner
        .iter()
        .zip(path.inner.iter())
        .all(|(a, b)| same(a, b))
    {
        Some(path.slice_from(prefix.len()))
    } else {
        None
    }
}
