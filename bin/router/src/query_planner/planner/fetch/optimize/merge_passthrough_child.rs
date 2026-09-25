use std::collections::{HashMap, VecDeque};

use petgraph::{graph::NodeIndex, visit::EdgeRef, Direction};
use tracing::{instrument, trace};

use crate::query_planner::{
    ast::selection_set::selection_items_are_subset_of,
    planner::fetch::{
        error::FetchGraphError, fetch_graph::FetchGraph, fetch_step_data::FetchStepData,
    },
};

impl FetchGraph {
    /// When a child has the input identical as the output,
    /// it gets squashed into its parent.
    /// Its children becomes children of the parent.
    #[instrument(level = "trace", skip_all)]
    pub(crate) fn merge_passthrough_child(&mut self) -> Result<(), FetchGraphError> {
        let root_index = self
            .root_index
            .ok_or(FetchGraphError::NonSingleRootStep(0))?;
        // Breadth-First Search (BFS) starting from the root node.
        let mut queue = VecDeque::from([root_index]);
        // A child merged into its parent is the parent from then on.
        let mut merged_into: HashMap<NodeIndex, NodeIndex> = HashMap::new();

        while let Some(mut parent_index) = queue.pop_front() {
            while let Some(merged) = merged_into.get(&parent_index) {
                parent_index = *merged;
            }

            let children: Vec<_> = self
                .graph
                .neighbors_directed(parent_index, Direction::Outgoing)
                .collect();

            for child_index in children {
                queue.push_back(child_index);

                let parent = self.get_step_data(parent_index)?;
                let child = self.get_step_data(child_index)?;
                if parent.can_merge_passthrough_child(parent_index, child_index, child, self) {
                    trace!(
                        "passthrough optimization found: merge [{}] <-- [{}]",
                        parent_index.index(),
                        child_index.index()
                    );
                    perform_passthrough_child_merge(parent_index, child_index, self)?;
                    merged_into.insert(child_index, parent_index);
                }
            }
        }

        Ok(())
    }
}

impl FetchStepData {
    pub(crate) fn can_merge_passthrough_child(
        &self,
        self_index: NodeIndex,
        other_index: NodeIndex,
        other: &Self,
        fetch_graph: &FetchGraph,
    ) -> bool {
        if self_index == other_index {
            return false;
        }

        if other.input_rewrites.as_ref().is_some_and(|r| !r.is_empty()) {
            return false;
        }

        // if the `other` FetchStep has a single parent and it's `this` FetchStep
        if fetch_graph.parents_of(other_index).count() != 1 {
            return false;
        }

        if fetch_graph.parents_of(other_index).next().unwrap().source() != self_index {
            return false;
        }

        for (output_def_name, output_selections) in other.output.iter_selections() {
            if let Some(input_selections) = other.input.selections_for_definition(output_def_name) {
                if selection_items_are_subset_of(&input_selections.items, &output_selections.items)
                {
                    return true;
                }
            }
        }

        false
    }
}

#[instrument(level = "trace", skip_all)]
fn perform_passthrough_child_merge(
    self_index: NodeIndex,
    other_index: NodeIndex,
    fetch_graph: &mut FetchGraph,
) -> Result<(), FetchGraphError> {
    let (me, other) = fetch_graph.get_pair_of_steps_mut(self_index, other_index)?;
    let path = other
        .response_path
        .strip_prefix(&me.response_path)
        .ok_or(FetchGraphError::MismatchedResponsePath)?;

    trace!(
        "merging fetch steps [{}] + [{}] at path {}",
        self_index.index(),
        other_index.index(),
        path
    );

    me.output.migrate_from_another(&other.output, &path)?;
    fetch_graph.replace_step(other_index, self_index);

    Ok(())
}
