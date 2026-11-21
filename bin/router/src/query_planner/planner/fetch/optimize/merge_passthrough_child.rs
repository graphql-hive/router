use std::collections::{HashSet, VecDeque};

use petgraph::{
    graph::NodeIndex,
    visit::{EdgeRef, NodeRef},
    Direction,
};
use tracing::{instrument, trace};

use crate::query_planner::{
    ast::selection_set::selection_items_are_subset_of,
    planner::fetch::{
        error::FetchGraphError,
        fetch_graph::FetchGraph,
        fetch_step_data::{FetchStepData, FetchStepFlags},
        optimize::utils::MergedSteps,
        state::MultiTypeFetchStep,
    },
};

impl FetchGraph<MultiTypeFetchStep> {
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
        let mut merged_steps = MergedSteps::default();
        // A step with several parents gets queued once per parent. Checking it again only finds
        // something new when a child was merged into it since, so the rest are skipped.
        let mut checked = HashSet::new();

        while let Some(parent_index) = queue.pop_front() {
            // Store pairs of sibling nodes that can be merged.
            let mut merges_to_perform: Vec<(NodeIndex, NodeIndex)> = Vec::new();
            let parent_index = merged_steps.resolve(parent_index);
            if !checked.insert(parent_index) {
                continue;
            }

            let children: Vec<_> = self
                .graph
                .neighbors_directed(parent_index, Direction::Outgoing)
                .collect();

            let parent = self.get_step_data(parent_index)?;

            for child_index in children.iter() {
                queue.push_back(*child_index);
                // Add the current child to the queue for further processing (BFS).
                let child = self.get_step_data(*child_index)?;

                if parent.can_merge_passthrough_child(parent_index, *child_index, child, self) {
                    trace!(
                        "passthrough optimization found: merge [{}] <-- [{}]",
                        parent_index.index(),
                        child_index.index()
                    );
                    merges_to_perform.push((parent_index, *child_index));
                }
            }

            for (parent_index, child_index) in merges_to_perform {
                let parent_index = merged_steps.resolve(parent_index);
                let child_index = merged_steps.resolve(child_index);
                // An earlier merge may have joined them already, or changed one of them.
                if parent_index == child_index {
                    continue;
                }
                let parent = self.get_step_data(parent_index)?;
                let child = self.get_step_data(child_index)?;
                if !parent.can_merge_passthrough_child(parent_index, child_index, child, self) {
                    continue;
                }

                perform_passthrough_child_merge(parent_index, child_index, self)?;
                merged_steps.record(child_index, parent_index);
                // the child's queued entry leads back here, to check the children it brought
                checked.remove(&parent_index);
            }
        }

        Ok(())
    }

    /// A step that only returns what's in its input, with more than one parent, adds nothing to
    /// the response: its parents put its input there. It goes away and its children wait for its
    /// parents instead. One parent is `merge_passthrough_child`'s job. Steps without children are
    /// left to the other passes.
    #[instrument(level = "trace", skip_all)]
    pub(crate) fn drop_passthrough_steps(&mut self) -> Result<(), FetchGraphError> {
        let root_index = self.root_index;
        let passthrough: Vec<NodeIndex> = self
            .graph
            .node_indices()
            .filter(|&index| Some(index) != root_index)
            .filter(|&index| {
                self.parents_of(index).count() > 1 && self.children_of(index).next().is_some()
            })
            .filter(|&index| {
                let step = &self.graph[index];
                step.input_rewrites.is_none()
                    && step.output_rewrites.is_none()
                    && step.context_rewrites.is_none()
                    && step.context_variable_definitions.is_empty()
                    && step.context_producers.is_empty()
                    && !step.flags.contains(FetchStepFlags::USED_FOR_TYPE_CONDITION)
                    && step.output.iter_selections().all(|(definition, output)| {
                        step.input
                            .selections_for_definition(definition)
                            .is_some_and(|input| {
                                selection_items_are_subset_of(&input.items, &output.items)
                            })
                    })
            })
            .collect();
        for index in passthrough {
            let id = self.get_step_data(index)?.id;
            if self
                .graph
                .node_weights()
                .any(|step| step.context_producers.contains(&id))
            {
                continue;
            }
            let parents: Vec<NodeIndex> = self.parents_of(index).map(|e| e.source()).collect();
            let children: Vec<NodeIndex> = self.children_of(index).map(|e| e.target()).collect();
            trace!("dropping passthrough step [{}]", index.index());
            for &parent in &parents {
                for &child in &children {
                    self.connect(parent, child);
                }
            }
            self.remove_step(index);
        }
        Ok(())
    }
}

impl FetchStepData<MultiTypeFetchStep> {
    pub(crate) fn can_merge_passthrough_child(
        &self,
        self_index: NodeIndex,
        other_index: NodeIndex,
        other: &Self,
        fetch_graph: &FetchGraph<MultiTypeFetchStep>,
    ) -> bool {
        if !self.can_merge_context(other) {
            return false;
        }

        if self_index == other_index {
            return false;
        }

        if other
            .flags
            .contains(FetchStepFlags::USED_FOR_TYPE_CONDITION)
        {
            return false;
        }

        // if the `other` FetchStep has a single parent and it's `this` FetchStep
        if fetch_graph.parents_of(other_index).count() != 1 {
            return false;
        }

        if fetch_graph.parents_of(other_index).next().unwrap().source() != self_index {
            return false;
        }

        // with several types of our own, the path has to say which one it's under
        if self.place_of_nested(&other.response_path).is_none() {
            return false;
        }

        // Every type it fetches has to be in its input already, not just one of them.
        other
            .output
            .iter_selections()
            .all(|(output_def_name, output_selections)| {
                other
                    .input
                    .selections_for_definition(output_def_name)
                    .is_some_and(|input_selections| {
                        selection_items_are_subset_of(
                            &input_selections.items,
                            &output_selections.items,
                        )
                    })
            })
    }
}

#[instrument(level = "trace", skip_all)]
fn perform_passthrough_child_merge(
    self_index: NodeIndex,
    other_index: NodeIndex,
    fetch_graph: &mut FetchGraph<MultiTypeFetchStep>,
) -> Result<(), FetchGraphError> {
    let (me, other) = fetch_graph.get_pair_of_steps_mut(self_index, other_index)?;
    let (type_name, path) = me
        .place_of_nested(&other.response_path)
        .ok_or(FetchGraphError::MismatchedResponsePath)?;

    trace!(
        "merging fetch steps [{}] + [{}] at path {}",
        self_index.index(),
        other_index.index(),
        path
    );

    match type_name {
        Some(type_name) => me.output.migrate_into(&type_name, &other.output, &path)?,
        None => me.output.migrate_from_another(&other.output, &path)?,
    }
    me.merge_context_metadata(other);
    let source_id = other.id;
    let target_id = me.id;

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
    fetch_graph.replace_context_producer(source_id, target_id);
    fetch_graph.remove_step(other_index);

    Ok(())
}
