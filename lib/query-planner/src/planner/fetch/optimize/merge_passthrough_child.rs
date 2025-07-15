use std::collections::{HashMap, VecDeque};

use petgraph::{
    graph::NodeIndex,
    visit::{EdgeRef, NodeRef},
    Direction,
};
use tracing::{instrument, trace};

use crate::planner::fetch::{
    error::FetchGraphError, fetch_graph::FetchGraph, fetch_step_data::FetchStepData,
};

impl FetchGraph {
    /// When a child has the input contains the output,
    /// it gets squashed into its parent.
    /// Its children becomes children of the parent.
    #[instrument(level = "trace", skip_all)]
    pub(crate) fn merge_passthrough_child(&mut self) -> Result<(), FetchGraphError> {
        let root_index = self
            .root_index
            .ok_or(FetchGraphError::NonSingleRootStep(0))?;
        // Breadth-First Search (BFS) starting from the root node.
        let mut queue = VecDeque::from([root_index]);
        // HashMap to keep track of node index mappings, especially after merges.
        // Key: original index, Value: potentially updated index after merges.
        let mut node_indexes: HashMap<NodeIndex, NodeIndex> = HashMap::new();

        node_indexes.insert(root_index, root_index);

        while let Some(parent_index) = queue.pop_front() {
            // Store pairs of sibling nodes that can be merged.
            let mut merges_to_perform: Vec<(NodeIndex, NodeIndex)> = Vec::new();
            let parent_index = *node_indexes
                .get(&parent_index)
                .ok_or(FetchGraphError::IndexMappingLost)?;

            let children: Vec<_> = self
                .graph
                .neighbors_directed(parent_index, Direction::Outgoing)
                .collect();

            let parent = self.get_step_data(parent_index)?;

            for child_index in children.iter() {
                queue.push_back(*child_index);
                // Add the current child to the queue for further processing (BFS).
                let child = self.get_step_data(*child_index)?;
                node_indexes.insert(*child_index, *child_index);
                node_indexes.insert(parent_index, parent_index);

                if parent.can_merge_passthrough_child(parent_index, *child_index, child, self) {
                    trace!(
                        "passthrough optimization found: merge [{}] <-- [{}]",
                        parent_index.index(),
                        child_index.index()
                    );
                    // Register their original indexes in the map.
                    merges_to_perform.push((parent_index, *child_index));
                }
            }

            for (parent_index, child_index) in merges_to_perform {
                // Get the latest indexes for the nodes, accounting for previous merges.
                let parent_index_latest = node_indexes
                    .get(&parent_index)
                    .ok_or(FetchGraphError::IndexMappingLost)?;
                let child_index_latest = node_indexes
                    .get(&child_index)
                    .ok_or(FetchGraphError::IndexMappingLost)?;

                perform_passthrough_child_merge(*parent_index_latest, *child_index_latest, self)?;

                // Because `child` was merged into `parent`,
                // then everything that was pointing to `child`
                // has to point to the `parent`.
                node_indexes.insert(*child_index_latest, *parent_index_latest);
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

        // if the `other` FetchStep has a single parent and it's `this` FetchStep
        if fetch_graph.parents_of(other_index).count() != 1 {
            return false;
        }

        if fetch_graph.parents_of(other_index).next().unwrap().source() != self_index {
            return false;
        }

        other.input.contains(&other.output)
    }
}

#[instrument(level = "trace", skip_all)]
fn perform_passthrough_child_merge(
    self_index: NodeIndex,
    other_index: NodeIndex,
    fetch_graph: &mut FetchGraph,
) -> Result<(), FetchGraphError> {
    let (me, other) = fetch_graph.get_pair_of_steps_mut(self_index, other_index)?;

    trace!(
        "merging fetch steps [{}] + [{}]",
        self_index.index(),
        other_index.index()
    );

    me.output.add_at_path(
        &other.output,
        other.response_path.slice_from(me.response_path.len()),
        false,
    )?;

    let mut children_indexes: Vec<NodeIndex> = vec![];
    let mut parents_indexes: Vec<NodeIndex> = vec![];
    for edge_ref in fetch_graph.children_of(other_index) {
        children_indexes.push(edge_ref.target().id());
    }

    for edge_ref in fetch_graph.parents_of(other_index) {
        // We ignore self_index
        if edge_ref.source().id() != self_index {
            parents_indexes.push(edge_ref.source().id());
        }
    }

    // Replace parents:
    // 1. Add self -> child
    for child_index in children_indexes {
        trace!(
            "migrating parent [{}] to child [{}]",
            self_index.index(),
            child_index.index()
        );

        fetch_graph.connect(self_index, child_index);
    }

    // 2. Add parent -> self
    for parent_index in parents_indexes {
        trace!(
            "linking parent [{}] to self [{}]",
            parent_index.index(),
            self_index.index()
        );

        fetch_graph.connect(parent_index, self_index);
    }

    // 3. Drop other -> child and parent -> other
    trace!("removing other [{}] from graph", other_index.index());
    fetch_graph.remove_step(other_index);

    Ok(())
}
