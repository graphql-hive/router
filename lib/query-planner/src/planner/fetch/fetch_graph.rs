use crate::ast::merge_path::{MergePath, Segment};
use crate::ast::operation::VariableDefinition;
use crate::ast::selection_item::SelectionItem;
use crate::ast::selection_set::{FieldSelection, InlineFragmentSelection, SelectionSet};
use crate::ast::type_aware_selection::TypeAwareSelection;
use crate::graph::edge::{Edge, FieldMove, InterfaceObjectTypeMove};
use crate::graph::node::Node;
use crate::graph::Graph;
use crate::planner::plan_nodes::{FetchRewrite, ValueSetter};
use crate::planner::tree::query_tree::QueryTree;
use crate::planner::tree::query_tree_node::{MutationFieldPosition, QueryTreeNode};
use crate::planner::walker::path::OperationPath;
use crate::planner::walker::pathfinder::can_satisfy_edge;
use crate::state::supergraph_state::SubgraphName;
use petgraph::graph::EdgeReference;
use petgraph::stable_graph::{EdgeIndex, NodeIndex, NodeIndices, NodeReferences, StableDiGraph};
use petgraph::visit::{Bfs, IntoNodeReferences};
use petgraph::visit::{EdgeRef, NodeRef};
use petgraph::Directed;
use petgraph::Direction;
use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::fmt::{Debug, Display};
use tracing::{instrument, trace};

use super::error::FetchGraphError;

#[derive(Debug, Clone)]
pub struct FetchGraph {
    graph: StableDiGraph<FetchStepData, ()>,
    pub root_index: Option<NodeIndex>,
}

impl Default for FetchGraph {
    fn default() -> Self {
        Self::new()
    }
}

impl FetchGraph {
    pub fn new() -> Self {
        Self {
            graph: StableDiGraph::new(),
            root_index: None,
        }
    }

    pub fn all_nodes(&self) -> NodeReferences<'_, FetchStepData> {
        self.graph.node_references()
    }
}

type MergesSiblingsToPerform = Vec<(NodeIndex, NodeIndex, Option<Vec<(usize, usize)>>)>;

impl FetchGraph {
    pub fn parents_of(&self, index: NodeIndex) -> petgraph::stable_graph::Edges<'_, (), Directed> {
        self.graph.edges_directed(index, Direction::Incoming)
    }

    pub fn children_of(&self, index: NodeIndex) -> petgraph::stable_graph::Edges<'_, (), Directed> {
        self.graph.edges_directed(index, Direction::Outgoing)
    }

    pub fn step_indices(&self) -> NodeIndices<FetchStepData> {
        self.graph.node_indices()
    }

    pub fn get_step_data(&self, index: NodeIndex) -> Result<&FetchStepData, FetchGraphError> {
        self.graph
            .node_weight(index)
            .ok_or(FetchGraphError::MissingStep(
                index.index(),
                String::from("when getting step data"),
            ))
    }

    pub fn get_step_data_mut(
        &mut self,
        index: NodeIndex,
    ) -> Result<&mut FetchStepData, FetchGraphError> {
        self.graph
            .node_weight_mut(index)
            .ok_or(FetchGraphError::MissingStep(
                index.index(),
                String::from("when getting mutable step data"),
            ))
    }

    pub fn get_pair_of_steps_mut(
        &mut self,
        index1: NodeIndex,
        index2: NodeIndex,
    ) -> Result<(&mut FetchStepData, &mut FetchStepData), FetchGraphError> {
        // `index_twice_mut` panics when indexes are equal
        if index1 == index2 {
            return Err(FetchGraphError::SameNodeIndex(index1.index()));
        }

        // `index_twice_mut` panics when nodes do not exist
        if self.graph.node_weight(index1).is_none() {
            return Err(FetchGraphError::MissingStep(
                index1.index(),
                String::from("when checking existence"),
            ));
        }
        if self.graph.node_weight(index2).is_none() {
            return Err(FetchGraphError::MissingStep(
                index2.index(),
                String::from("when checking existence"),
            ));
        }

        Ok(self.graph.index_twice_mut(index1, index2))
    }

    #[instrument(level = "trace",skip_all, fields(
      parent = parent_index.index(),
      child = child_index.index(),
    ))]
    pub fn connect(&mut self, parent_index: NodeIndex, child_index: NodeIndex) -> EdgeIndex {
        self.graph.update_edge(parent_index, child_index, ())
    }

    pub fn remove_edge(&mut self, edge_index: EdgeIndex) -> bool {
        self.graph.remove_edge(edge_index).is_some_and(|_| true)
    }

    #[instrument(level = "trace", skip_all, fields(
      index = index.index(),
    ))]
    pub fn remove_step(&mut self, index: NodeIndex) -> bool {
        self.graph.remove_node(index).is_some_and(|_| true)
    }

    pub fn add_step(&mut self, data: FetchStepData) -> NodeIndex {
        self.graph.add_node(data)
    }

    pub fn bfs<F>(&self, root_index: NodeIndex, mut visitor: F) -> Option<NodeIndex>
    where
        F: FnMut(&NodeIndex, &FetchStepData) -> bool,
    {
        self.graph.node_weight(root_index)?;

        let mut bfs = Bfs::new(&self.graph, root_index);

        while let Some(step_index) = bfs.next(&self.graph) {
            // Get the data for the current step. bfs.next() should yield valid indices.
            let step_data = self
                .graph
                .node_weight(step_index)
                .expect("BFS returned invalid node index");

            if visitor(&step_index, step_data) {
                return Some(step_index);
            }
        }

        None
    }

    #[instrument(level = "trace", skip_all)]
    pub fn collect_variable_usages(&mut self) -> Result<(), FetchGraphError> {
        let nodes_idx = self.graph.node_indices().collect::<Vec<_>>();

        for node_idx in nodes_idx {
            let step_data = self.get_step_data_mut(node_idx)?;
            let usage = step_data.output.selection_set.variable_usages();

            if !usage.is_empty() {
                step_data.variable_usages = Some(usage);
            }
        }

        Ok(())
    }

    #[instrument(level = "trace", skip_all)]
    pub fn optimize(&mut self) -> Result<(), FetchGraphError> {
        self.merge_passthrough_child()?;
        self.merge_children_with_parents()?;
        self.merge_siblings()?;
        self.deduplicate_and_prune_fetch_steps()?;
        self.apply_internal_aliases_patching()?;
        self.turn_mutations_into_sequence()?;

        Ok(())
    }

    /// Removes redundant direct dependencies from a FetchStep graph.
    ///
    /// ```text
    /// in:
    /// A -> C
    /// A -> B -> ... -> C
    /// out:
    /// A -> B -> ... -> C
    /// ```
    #[instrument(level = "trace", skip_all)]
    fn deduplicate_and_prune_fetch_steps(&mut self) -> Result<(), FetchGraphError> {
        let steps_to_remove: Vec<NodeIndex> = self
            .step_indices()
            .filter(|&step_index| {
                let step = match self.get_step_data(step_index) {
                    Ok(s) => s,
                    Err(_) => return false,
                };

                if !step.output.selection_set.items.is_empty()
                    && self.parents_of(step_index).next().is_some()
                {
                    return false;
                }

                if self.children_of(step_index).next().is_some() {
                    return false;
                }

                trace!("optimization found: remove '{}'", step);

                true
            })
            .collect();

        for step_index in steps_to_remove {
            self.remove_step(step_index);
        }

        let mut edges_to_remove: Vec<EdgeIndex> = vec![];
        for step_index in self.step_indices() {
            for parent_to_step_edge in self.parents_of(step_index) {
                let direct_parent_index = parent_to_step_edge.source();
                let child_index = step_index;
                if is_reachable_via_alternative_upstream_path(
                    self,
                    child_index,
                    direct_parent_index,
                )? {
                    edges_to_remove.push(parent_to_step_edge.id());
                }
            }
        }

        for edge_index in edges_to_remove {
            self.remove_edge(edge_index);
        }

        Ok(())
    }

    #[instrument(level = "trace", skip_all)]
    fn apply_internal_aliases_patching(&mut self) -> Result<(), FetchGraphError> {
        // First, iterate and find all nodes that needed to perform internal aliasing for fields
        let mut nodes_with_aliases = self
            .graph
            .node_references()
            .filter_map(|(index, node)| {
                if !node.aliased_fields.is_empty() {
                    Some(index)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();

        trace!(
            "found total of {} node with internal aliased fields: {:?}",
            nodes_with_aliases.len(),
            nodes_with_aliases
        );

        // A list of modifications that needs to be applied as input rewrites.
        // Data is: (index_to_modify, field_index, alias_name)
        let mut nodes_that_needs_rewrite: Vec<(NodeIndex, usize, String)> = vec![];
        // Data is: (index_to_modify, segment_index_in_response_path, alias_name)
        let mut nodes_that_needs_response_path_patching: Vec<(NodeIndex, usize, String)> = vec![];

        // For every node that has internal aliased fields...
        while let Some(aliased_node_index) = nodes_with_aliases.pop() {
            let node_with_aliases = self.get_step_data(aliased_node_index)?;
            let mut bfs = Bfs::new(&self.graph, aliased_node_index);

            // Iterate and find all possible children of a node that needed aliasing
            // We can't really tell which nodes are affected, as they might be at any level of the hierarchy.
            while let Some(decendent_idx) = bfs.next(&self.graph) {
                if decendent_idx != aliased_node_index {
                    let decendent = self.get_step_data(decendent_idx)?;

                    // First, let's try to find all response_path that's using this specific field as input.
                    // A segment in the response_path is identified by the selection_identifier + the arguments hash.
                    // Once we find it, we store it and we'll do patching later.
                    for (segment_index, segment_value) in
                        decendent.response_path.inner.iter().enumerate()
                    {
                        if let Segment::Field(selection_identifier, args_hash) = segment_value {
                            if let Some(aliased_field_name) = node_with_aliases
                                .aliased_fields
                                .get(&(selection_identifier.clone(), *args_hash))
                            {
                                nodes_that_needs_response_path_patching.push((
                                    decendent_idx,
                                    segment_index,
                                    aliased_field_name.clone(),
                                ));
                            }
                        }
                    }

                    // Next, iterate and find all nodes that needed to perform internal aliasing for fields
                    for (input_selection_item_idx, input_selection_item) in
                        decendent.input.selection_set.items.iter().enumerate()
                    {
                        if let SelectionItem::Field(field) = input_selection_item {
                            let key_tuple = (field.name.clone(), field.arguments_hash());

                            if let Some(alias_field_name) =
                                node_with_aliases.aliased_fields.get(&key_tuple)
                            {
                                trace!(
                                  "found a field '{}' (args hash: {}) that's using an aliased field: '{}', it need an input selection alias",
                                  key_tuple.0, key_tuple.1, alias_field_name
                              );
                                nodes_that_needs_rewrite.push((
                                    decendent_idx,
                                    input_selection_item_idx,
                                    alias_field_name.to_string(),
                                ));
                            }
                        }
                    }
                }
            }
        }

        // Go over all nodes that need to be rewritten, and add the alias in the right place
        for (idx_to_modify, field_idx, alias_name) in nodes_that_needs_rewrite {
            let step_data = self.get_step_data_mut(idx_to_modify)?;

            if let Some(SelectionItem::Field(field_to_mutate)) =
                step_data.input.selection_set.items.get_mut(field_idx)
            {
                trace!(
                    "patching input selection field step [{}]: {} -> {}",
                    idx_to_modify.index(),
                    field_to_mutate.name,
                    alias_name
                );

                field_to_mutate.alias = Some(field_to_mutate.name.clone());
                field_to_mutate.name = alias_name;
            }
        }

        // Go over all nodes that are using this field as input, and apply the patch on the response_path
        for (idx_to_modify, segment_idx, alias_name) in nodes_that_needs_response_path_patching {
            let step_data = self.get_step_data_mut(idx_to_modify)?;
            let mut new_path = (*step_data.response_path.inner).to_vec();

            if let Some(Segment::Field(name, _)) = new_path.get_mut(segment_idx) {
                trace!(
                    "patching response_path#{} on step [{}]: {name} -> {alias_name}",
                    segment_idx,
                    idx_to_modify.index()
                );

                *name = alias_name;
            }

            step_data.response_path = MergePath::new(new_path);
        }

        Ok(())
    }

    #[instrument(level = "trace", skip_all)]
    fn merge_siblings(&mut self) -> Result<(), FetchGraphError> {
        let root_index = self
            .root_index
            .ok_or(FetchGraphError::NonSingleRootStep(0))?;
        // Breadth-First Search (BFS) starting from the root node.
        let mut queue = VecDeque::from([root_index]);

        while let Some(parent_index) = queue.pop_front() {
            // Store pairs of sibling nodes that can be merged.
            // The additional Vec<usize> is an indicator for conflicting field indexes in the 2nd sibling.
            // If the Vec is empty, it means there are no conflicts.
            let mut merges_to_perform = MergesSiblingsToPerform::new();

            // HashMap to keep track of node index mappings, especially after merges.
            // Key: original index, Value: potentially updated index after merges.
            let mut node_indexes: HashMap<NodeIndex, NodeIndex> = HashMap::new();

            // Sort fetch steps by mutation's field position,
            // to execute mutations in correct order.
            let mut siblings_with_pos: Vec<(NodeIndex, Option<usize>)> = self
                .graph
                .neighbors_directed(parent_index, Direction::Outgoing)
                .map(|sibling| {
                    self.get_step_data(sibling)
                        .map(|data| (sibling, data.mutation_field_position))
                })
                .collect::<Result<_, _>>()?;

            // Sort fetch steps by mutation's field position,
            // to execute mutations in correct order.
            siblings_with_pos.sort_by_key(|(_node, pos)| *pos);

            let siblings: Vec<NodeIndex> =
                siblings_with_pos.into_iter().map(|(idx, _)| idx).collect();

            for (i, sibling_index) in siblings.iter().enumerate() {
                // Add the current node to the queue for further processing (BFS).
                queue.push_back(*sibling_index);
                let current = self.get_step_data(*sibling_index)?;

                // Iterate through the remaining children (siblings) to check for merge possibilities.
                for other_sibling_index in siblings.iter().skip(i + 1) {
                    let other_sibling = self.get_step_data(*other_sibling_index)?;

                    trace!(
                        "checking if [{}] and [{}] can be merged",
                        sibling_index.index(),
                        other_sibling_index.index()
                    );

                    let merge_siblings_result = current.can_merge_siblings(
                        *sibling_index,
                        *other_sibling_index,
                        other_sibling,
                        self,
                    );

                    // Siblings can be merged if they have no input conflicts only. In case of input conflicts, we cannot perform any kind of merge.
                    let can_merge = merge_siblings_result
                        .as_ref()
                        .is_some_and(|result| !result.has_input_conflicts());

                    if can_merge {
                        let conflicting_output_fields = merge_siblings_result
                            .filter(|result| !result.conflicting_output_fields.is_empty())
                            .map(|result| result.conflicting_output_fields);

                        trace!(
                            "Found siblings optimization: {} <- {}",
                            sibling_index.index(),
                            other_sibling_index.index()
                        );
                        // Register their original indexes in the map.
                        node_indexes.insert(*sibling_index, *sibling_index);
                        node_indexes.insert(*other_sibling_index, *other_sibling_index);

                        merges_to_perform.push((
                            *sibling_index,
                            *other_sibling_index,
                            conflicting_output_fields,
                        ));

                        // Since a merge is possible, move to the next child to avoid redundant checks.
                        break;
                    }
                }
            }

            for (child_index, other_child_index, maybe_field_conflicts) in merges_to_perform {
                // Get the latest indexes for the nodes, accounting for previous merges.
                let child_index_latest = node_indexes
                    .get(&child_index)
                    .ok_or(FetchGraphError::IndexMappingLost)?;
                let other_child_index_latest = node_indexes
                    .get(&other_child_index)
                    .ok_or(FetchGraphError::IndexMappingLost)?;

                // If any conflicts exist, perform aliasing.
                if let Some(output_conflicts) = maybe_field_conflicts {
                    perform_aliasing_for_conflicts(
                        *child_index_latest,
                        *other_child_index_latest,
                        &output_conflicts,
                        self,
                    )?;
                }

                perform_fetch_step_merge(*child_index_latest, *other_child_index_latest, self)?;

                // Because `other_child` was merged into `child`,
                // then everything that was pointing to `other_child`
                // has to point to the `child`.
                node_indexes.insert(*other_child_index_latest, *child_index_latest);
            }
        }
        Ok(())
    }

    /// When a child has the input identical as the output,
    /// it gets squashed into its parent.
    /// Its children becomes children of the parent.
    #[instrument(level = "trace", skip_all)]
    fn merge_passthrough_child(&mut self) -> Result<(), FetchGraphError> {
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
                .expect("Index mapping got lost");

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
                    .expect("Index mapping got lost");
                let child_index_latest = node_indexes
                    .get(&child_index)
                    .expect("Index mapping got lost");

                perform_passthrough_child_merge(*parent_index_latest, *child_index_latest, self)?;

                // Because `child` was merged into `parent`,
                // then everything that was pointing to `child`
                // has to point to the `parent`.
                node_indexes.insert(*child_index_latest, *parent_index_latest);
            }
        }

        Ok(())
    }

    #[instrument(level = "trace", skip_all)]
    fn merge_children_with_parents(&mut self) -> Result<(), FetchGraphError> {
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
                .expect("Index mapping got lost");

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

                if parent.can_merge(parent_index, *child_index, child, self) {
                    trace!(
                        "optimization found: merge parent [{}] with child [{}]",
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
                    .expect("Index mapping got lost");
                let child_index_latest = node_indexes
                    .get(&child_index)
                    .expect("Index mapping got lost");

                perform_fetch_step_merge(*parent_index_latest, *child_index_latest, self)?;

                // Because `child` was merged into `parent`,
                // then everything that was pointing to `child`
                // has to point to the `parent`.
                node_indexes.insert(*child_index_latest, *parent_index_latest);
            }
        }

        Ok(())
    }

    #[instrument(level = "trace", skip_all)]
    fn turn_mutations_into_sequence(&mut self) -> Result<(), FetchGraphError> {
        let root_index = self
            .root_index
            .ok_or(FetchGraphError::NonSingleRootStep(0))?;
        if !is_mutation_fetch_step(self, root_index)? {
            return Ok(());
        }

        let mut node_mutation_field_pos_pairs: Vec<(NodeIndex, usize)> = Vec::new();
        let mut edge_ids_to_remove: Vec<EdgeIndex> = Vec::new();

        for edge_ref in self.children_of(root_index) {
            edge_ids_to_remove.push(edge_ref.id());
            let node_index = edge_ref.target().id();
            let mutation_field_pos = self
                .get_step_data(node_index)?
                .mutation_field_position
                .ok_or(FetchGraphError::MutationStepWithNoOrder)?;
            node_mutation_field_pos_pairs.push((node_index, mutation_field_pos));
        }

        node_mutation_field_pos_pairs.sort_by_key(|&(_, pos)| pos);

        let mut new_edges_pairs: Vec<(NodeIndex, NodeIndex)> = Vec::new();
        let mut iter = node_mutation_field_pos_pairs.iter();
        let mut current = iter.next();

        for next_sequence_child in iter {
            if let Some((current_node_index, _pos)) = current {
                let next_node_index = next_sequence_child.0;
                new_edges_pairs.push((current_node_index.id(), next_node_index));
            }
            current = Some(next_sequence_child);
        }

        for edge_id in edge_ids_to_remove {
            self.remove_edge(edge_id);
        }

        // Bring back the root -> Mutation edge
        let first_pair = node_mutation_field_pos_pairs
            .first()
            .ok_or(FetchGraphError::EmptyFetchSteps)?;
        self.connect(root_index, first_pair.0);

        for (from_id, to_id) in new_edges_pairs {
            self.connect(from_id, to_id);
        }

        Ok(())
    }
}

impl Display for FetchGraph {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Nodes:")?;
        for node_index in self.graph.node_indices() {
            // ignore root node
            if node_index.index() == 0 {
                continue;
            }
            if let Some(fetch_step) = self.graph.node_weight(node_index) {
                fetch_step.pretty_write(f, node_index)?;
                writeln!(f)?;
            }
        }

        writeln!(f, "\nTree:")?;

        let mut stack: VecDeque<(NodeIndex, usize)> = VecDeque::new();

        let roots = find_graph_roots(self);

        if roots.is_empty() {
            writeln!(f, "Fetch step graph is empty or has no roots.")?;
            return Ok(());
        }

        if roots.len() > 1 {
            writeln!(f, "Fetch step graph has multiple roots:")?;
            return Ok(());
        }

        for root_index in roots {
            for child_index in self
                .graph
                .edges_directed(root_index, Direction::Outgoing)
                .map(|edge_ref| edge_ref.target())
            {
                stack.push_front((child_index, 0));
            }
        }

        while let Some((node_index, depth)) = stack.pop_back() {
            let indent = "  ".repeat(depth);
            writeln!(f, "{indent}[{}]", node_index.index())?;

            for child_index in self
                .graph
                .edges_directed(node_index, Direction::Outgoing)
                .map(|edge_ref| edge_ref.target())
            {
                stack.push_back((child_index, depth + 1));
            }
        }

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum FetchStepKind {
    Entity,
    Root,
}

// A map between: (selection_identifier, arguments_hash) -> alias_name
//
// ## Key
// The "selection_identifier" is the alias, or the selection name.
// The "arguments_hash" is the hash of the arguments passed to the field.
// We are using both together as a key to uniquely identify a field, and to check if it needs to be aliased in the plan.
// ## Value
// The value of the map is the new name of the alias.
// The data stored in the value of this map is later used to create aliases on nodes that depend on the one that was originally patched.
// Every internal alias made, will follow with an selection alias + response_path alias, in one or many of the children fetch steps (at any level).
pub type InternalAliasMap = HashMap<(String, u64), String>;

#[derive(Debug, Clone)]
pub struct FetchStepData {
    pub service_name: SubgraphName,
    pub response_path: MergePath,
    pub input: TypeAwareSelection,
    pub output: TypeAwareSelection,
    pub kind: FetchStepKind,
    pub aliased_fields: InternalAliasMap,
    pub used_for_requires: bool,
    pub variable_usages: Option<BTreeSet<String>>,
    pub variable_definitions: Option<Vec<VariableDefinition>>,
    pub mutation_field_position: MutationFieldPosition,
    pub input_rewrites: Option<Vec<FetchRewrite>>,
}

impl Display for FetchStepData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}/{} {} → {} at $.{}",
            self.input.type_name,
            self.service_name,
            self.input,
            self.output,
            self.response_path.join("."),
        )?;

        if self.used_for_requires {
            write!(f, " [@requires]")?;
        }

        if !self.aliased_fields.is_empty() {
            write!(f, " [aliases=")?;
            for (i, (alias, field)) in self.aliased_fields.iter().enumerate() {
                if i > 0 {
                    write!(f, ", ")?;
                }
                write!(f, "{}→{}", alias.0, field)?;
            }
            write!(f, "]")?;
        }

        Ok(())
    }
}

impl FetchStepData {
    fn next_alias_id(&self) -> usize {
        self.aliased_fields.len()
    }

    pub fn pretty_write(
        &self,
        writer: &mut std::fmt::Formatter<'_>,
        index: NodeIndex,
    ) -> Result<(), std::fmt::Error> {
        write!(writer, "[{}] {}", index.index(), self)
    }

    pub fn is_entity_call(&self) -> bool {
        self.input.type_name != "Query"
            && self.input.type_name != "Mutation"
            && self.input.type_name != "Subscription"
    }

    pub fn add_input_rewrite(&mut self, rewrite: FetchRewrite) {
        let rewrites = self.input_rewrites.get_or_insert_default();

        if !rewrites.contains(&rewrite) {
            rewrites.push(rewrite);
        }
    }

    /// see `perform_passthrough_child_merge`
    pub fn can_merge_passthrough_child(
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

        other.input.eq(&other.output)
    }

    pub fn can_merge_siblings(
        &self,
        self_index: NodeIndex,
        other_index: NodeIndex,
        other: &Self,
        fetch_graph: &FetchGraph,
    ) -> Option<CanMergeSiblingsResult> {
        // First, check if the base conditions for merging are met.
        let can_merge_base = self.can_merge(self_index, other_index, other, fetch_graph);

        if let (Some(self_mut_idx), Some(other_mut_index)) =
            (self.mutation_field_position, other.mutation_field_position)
        {
            // If indexes are equal or one happens to be after the other,
            // and we already know they belong to the same service,
            // we shouldn't prevent merging.
            if self_mut_idx != other_mut_index
                && (self_mut_idx as i64 - other_mut_index as i64).abs() != 1
            {
                return None;
            }
        }

        // Now that we think it can be merged,
        // let's validate the selection sets and apply specific rules to avoid conflicts when merging siblings.
        if can_merge_base {
            let input_conflicts = find_arguments_conflicts(&self.input, &other.input);
            let output_conflicts = find_arguments_conflicts(&other.output, &self.output);

            return Some(CanMergeSiblingsResult {
                conflicting_input_fields: input_conflicts,
                conflicting_output_fields: output_conflicts,
            });
        }

        None
    }

    pub fn can_merge(
        &self,
        self_index: NodeIndex,
        other_index: NodeIndex,
        other: &Self,
        fetch_graph: &FetchGraph,
    ) -> bool {
        if self_index == other_index {
            return false;
        }

        if self.service_name != other.service_name {
            return false;
        }

        // If both are entities, their response_paths should match,
        // as we can't merge entity calls resolving different entities
        if matches!(self.kind, FetchStepKind::Entity) && self.kind == other.kind {
            if !self.response_path.eq(&other.response_path) {
                return false;
            }
        } else {
            // otherwise we can merge
            if !other.response_path.starts_with(&self.response_path) {
                return false;
            }
        }

        // if the `other` FetchStep has a single parent and it's `this` FetchStep
        if fetch_graph.parents_of(other_index).count() == 1
            && fetch_graph
                .parents_of(other_index)
                .all(|edge| edge.source() == self_index)
        {
            return true;
        }

        // if they do not share parents, they can't be merged
        if !fetch_graph.parents_of(self_index).all(|self_edge| {
            fetch_graph
                .parents_of(other_index)
                .any(|other_edge| other_edge.source() == self_edge.source())
        }) {
            return false;
        }

        true
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
    );

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

#[instrument(level = "trace", skip_all)]
fn solve_output_conflicts_by_aliasing(
    step: &mut FetchStepData,
    field_index: usize,
    unique_alias_id: usize,
) {
    if let Some(SelectionItem::Field(field)) = step.output.selection_set.items.get_mut(field_index)
    {
        let new_alias = format!("_internal_qp_alias_{}", unique_alias_id);
        trace!(
            "adding alias for field {} (at index {}) -> {}",
            field.name,
            field_index,
            new_alias
        );

        let key_tuple = (
            field.selection_identifier().to_string(),
            field.arguments_hash(),
        );

        step.aliased_fields.insert(key_tuple, new_alias.to_string());
        field.alias = Some(new_alias);
    }
}

// Return the index of the modified step
#[instrument(level = "trace", skip_all)]
fn perform_aliasing_for_conflicts(
    me_index: NodeIndex,
    other_index: NodeIndex,
    conflicting_fields_pairs: &[(usize, usize)],
    fetch_graph: &mut FetchGraph,
) -> Result<NodeIndex, FetchGraphError> {
    trace!(
        "applying aliases to resolve {} conflict(s) between fetch steps [{}] <-> [{}]",
        conflicting_fields_pairs.len(),
        me_index.index(),
        other_index.index()
    );

    let (me, other) = fetch_graph.get_pair_of_steps_mut(me_index, other_index)?;

    // We need to decide which step to modify based on the used_for_requires flag.
    // If "me" is created from a "@requires" flow, it means we need to alias it, otherwise, we'll alias "other".
    // There's no such scenario where both steps are NOT created from "@requires" flows.
    let (step_to_modify_index, step_to_modify, indices_to_alias) =
        match (me.used_for_requires, other.used_for_requires) {
            (true, false) => (
                me_index,
                me,
                conflicting_fields_pairs
                    .iter()
                    .map(|r| r.0)
                    .collect::<Vec<usize>>(),
            ),
            (false, true) | (true, true) => (
                other_index,
                other,
                conflicting_fields_pairs
                    .iter()
                    .map(|r| r.1)
                    .collect::<Vec<usize>>(),
            ),
            (false, false) => return Err(FetchGraphError::UnexpectedConflict),
        };

    trace!(
        "decided to apply aliases to step [{}] because it's using requires",
        step_to_modify_index.index()
    );

    for index in indices_to_alias {
        let unique_alias_id = step_to_modify.next_alias_id();
        solve_output_conflicts_by_aliasing(step_to_modify, index, unique_alias_id);
    }

    Ok(step_to_modify_index)
}

// Return true in case an alias was applied during the merge process.
#[instrument(level = "trace", skip_all)]
fn perform_fetch_step_merge(
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
    );

    if me.input.type_name == other.input.type_name {
        if me.response_path != other.response_path {
            return Err(FetchGraphError::MismatchedResponsePath);
        }

        me.input.add(&other.input);
    }

    if !other.aliased_fields.is_empty() {
        me.aliased_fields.extend(other.aliased_fields.clone());
    }

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
    for child_index in children_indexes.iter() {
        fetch_graph.connect(self_index, *child_index);
    }
    // 2. Add parent -> self
    for parent_index in parents_indexes {
        fetch_graph.connect(parent_index, self_index);
    }
    // 3. Drop other -> child and parent -> other
    fetch_graph.remove_step(other_index);

    Ok(())
}

fn create_noop_fetch_step(fetch_graph: &mut FetchGraph, created_from_requires: bool) -> NodeIndex {
    fetch_graph.add_step(FetchStepData {
        service_name: SubgraphName::any(),
        response_path: MergePath::default(),
        input: TypeAwareSelection {
            selection_set: SelectionSet::default(),
            type_name: "*".to_string(),
        },
        output: TypeAwareSelection {
            selection_set: SelectionSet::default(),
            type_name: "*".to_string(),
        },
        used_for_requires: created_from_requires,
        kind: FetchStepKind::Root,
        aliased_fields: HashMap::new(),
        input_rewrites: None,
        variable_usages: None,
        variable_definitions: None,
        mutation_field_position: None,
    })
}

fn create_fetch_step_for_entity_call(
    fetch_graph: &mut FetchGraph,
    subgraph_name: &SubgraphName,
    input_type_name: &str,
    output_type_name: &str,
    response_path: &MergePath,
    used_for_requires: bool,
) -> NodeIndex {
    fetch_graph.add_step(FetchStepData {
        service_name: subgraph_name.clone(),
        response_path: response_path.clone(),
        input: TypeAwareSelection {
            selection_set: SelectionSet {
                items: vec![SelectionItem::Field(FieldSelection::new_typename())],
            },
            type_name: input_type_name.to_string(),
        },
        output: TypeAwareSelection {
            selection_set: SelectionSet::default(),
            type_name: output_type_name.to_string(),
        },
        used_for_requires,
        kind: FetchStepKind::Entity,
        aliased_fields: HashMap::new(),
        input_rewrites: None,
        variable_usages: None,
        variable_definitions: None,
        mutation_field_position: None,
    })
}

fn create_fetch_step_for_root_move(
    fetch_graph: &mut FetchGraph,
    root_step_index: NodeIndex,
    subgraph_name: &SubgraphName,
    type_name: &str,
    mutation_field_position: MutationFieldPosition,
) -> NodeIndex {
    let idx = fetch_graph.add_step(FetchStepData {
        service_name: subgraph_name.clone(),
        response_path: MergePath::default(),
        input: TypeAwareSelection {
            selection_set: SelectionSet::default(),
            type_name: type_name.to_string(),
        },
        output: TypeAwareSelection {
            selection_set: SelectionSet::default(),
            type_name: type_name.to_string(),
        },
        used_for_requires: false,
        kind: FetchStepKind::Root,
        aliased_fields: HashMap::new(),
        variable_usages: None,
        variable_definitions: None,
        input_rewrites: None,
        mutation_field_position,
    });

    fetch_graph.connect(root_step_index, idx);

    idx
}

// TODO: simplfy args
#[allow(clippy::too_many_arguments)]
fn ensure_fetch_step_for_subgraph(
    fetch_graph: &mut FetchGraph,
    parent_fetch_step_index: NodeIndex,
    subgraph_name: &SubgraphName,
    input_type_name: &str,
    output_type_name: &str,
    response_path: &MergePath,
    key: Option<&TypeAwareSelection>,
    requires: Option<&TypeAwareSelection>,
    created_from_requires: bool,
) -> Result<NodeIndex, FetchGraphError> {
    let matching_child_index = if requires.is_some() {
        None
    } else {
        fetch_graph
            .children_of(parent_fetch_step_index)
            .find_map(|to_child_edge_ref| {
                if let Ok(fetch_step) = fetch_graph.get_step_data(to_child_edge_ref.target()) {
                    if fetch_step.service_name != *subgraph_name {
                        return None;
                    }

                    if fetch_step.input.type_name != *input_type_name {
                        return None;
                    }

                    if fetch_step.response_path != *response_path {
                        return None;
                    }

                    if let Some(key) = &key {
                        if !fetch_step.input.contains(key) {
                            // requested key fields are not part of the input
                            return None;
                        }
                    }

                    // If there are requirements, then we do not re-use
                    // optimizations will try to re-use the existing step later, if possible.
                    if fetch_step.used_for_requires || requires.is_some() {
                        return None;
                    }

                    return Some(to_child_edge_ref.target());
                }

                None
            })
    };

    match matching_child_index {
        Some(idx) => {
            trace!(
                "found existing fetch step [{}] for entity move requirement({}) key({}) in children of {}",
                idx.index(),
                requires.map(|r| r.to_string()).unwrap_or_default(),
                key.map(|r| r.to_string()).unwrap_or_default(),
                parent_fetch_step_index.index(),
            );
            Ok(idx)
        }
        None => {
            let step_index = create_fetch_step_for_entity_call(
                fetch_graph,
                subgraph_name,
                input_type_name,
                output_type_name,
                response_path,
                created_from_requires || requires.is_some(),
            );
            if let Some(selection) = key {
                let step = fetch_graph.get_step_data_mut(step_index)?;
                step.input.add(selection)
            }

            trace!(
                "created a new fetch step [{}] subgraph({}) type({}) requirement({}) key({}) in children of {}",
                step_index.index(),
                subgraph_name,
                input_type_name,
                requires.map(|r| r.to_string()).unwrap_or_default(),
                key.map(|r| r.to_string()).unwrap_or_default(),
                parent_fetch_step_index.index(),
            );

            Ok(step_index)
        }
    }
}

fn ensure_fetch_step_for_requirement(
    fetch_graph: &mut FetchGraph,
    parent_fetch_step_index: NodeIndex,
    subgraph_name: &SubgraphName,
    type_name: &String,
    response_path: &MergePath,
    requirement: &TypeAwareSelection,
) -> Result<NodeIndex, FetchGraphError> {
    let matching_child_index =
        fetch_graph
            .children_of(parent_fetch_step_index)
            .find_map(|to_child_edge_ref| {
                if let Ok(fetch_step) = fetch_graph.get_step_data(to_child_edge_ref.target()) {
                    if fetch_step.service_name != *subgraph_name {
                        return None;
                    }

                    if fetch_step.input.type_name != *type_name {
                        return None;
                    }

                    if fetch_step.response_path != *response_path {
                        return None;
                    }

                    if !fetch_step.input.contains(requirement) {
                        return None;
                    }

                    return Some(to_child_edge_ref.target());
                }

                None
            });

    match matching_child_index {
        Some(idx) => {
            trace!(
                "found existing fetch step [{}] children of {}",
                idx.index(),
                parent_fetch_step_index.index(),
            );
            Ok(idx)
        }
        None => {
            let step_index = create_fetch_step_for_entity_call(
                fetch_graph,
                subgraph_name,
                type_name,
                type_name,
                response_path,
                true,
            );

            trace!(
                "created a new fetch step [{}] subgraph({}) type({}) requirement({}) in children of {}",
                step_index.index(),
                subgraph_name,
                type_name,
                requirement.to_string(),
                parent_fetch_step_index.index(),
            );

            Ok(step_index)
        }
    }
}

// TODO: simplfy args
#[allow(clippy::too_many_arguments)]
#[instrument(level = "trace", skip_all, fields(
  count = query_node.children.len(),
  parent_fetch_step_index = parent_fetch_step_index.index(),
  requiring_fetch_step_index = requiring_fetch_step_index.map(|s| s.index())
))]
fn process_children_for_fetch_steps(
    graph: &Graph,
    fetch_graph: &mut FetchGraph,
    query_node: &QueryTreeNode,
    parent_fetch_step_index: NodeIndex,
    response_path: &MergePath,
    fetch_path: &MergePath,
    requiring_fetch_step_index: Option<NodeIndex>,
    created_from_requires: bool,
) -> Result<Vec<NodeIndex>, FetchGraphError> {
    if query_node.children.is_empty() {
        return Ok(vec![parent_fetch_step_index]);
    }

    let mut leaf_fetch_step_indexes: Vec<NodeIndex> = vec![];
    for sub_step in query_node.children.iter() {
        leaf_fetch_step_indexes.extend(process_query_node(
            graph,
            fetch_graph,
            sub_step,
            Some(parent_fetch_step_index),
            response_path,
            fetch_path,
            requiring_fetch_step_index,
            created_from_requires,
        )?);
    }

    Ok(leaf_fetch_step_indexes)
}

#[instrument(level = "trace",skip_all, fields(
  count = query_node.requirements.len()
))]
fn process_requirements_for_fetch_steps(
    graph: &Graph,
    fetch_graph: &mut FetchGraph,
    query_node: &QueryTreeNode,
    parent_fetch_step_index: NodeIndex,
    requiring_fetch_step_index: Option<NodeIndex>,
    response_path: &MergePath,
    fetch_path: &MergePath,
) -> Result<(), FetchGraphError> {
    if query_node.requirements.is_empty() {
        return Ok(());
    }

    for req_query_node in query_node.requirements.iter() {
        process_query_node(
            graph,
            fetch_graph,
            req_query_node,
            Some(parent_fetch_step_index),
            response_path,
            fetch_path,
            requiring_fetch_step_index,
            true,
        )?;
        fetch_graph.connect(
            parent_fetch_step_index,
            requiring_fetch_step_index.ok_or(FetchGraphError::IndexNone)?,
        );
    }

    Ok(())
}

// TODO: simplfy args
#[allow(clippy::too_many_arguments)]
#[instrument(level = "trace", skip_all)]
fn process_noop_edge(
    graph: &Graph,
    fetch_graph: &mut FetchGraph,
    query_node: &QueryTreeNode,
    parent_fetch_step_index: Option<NodeIndex>,
    response_path: &MergePath,
    fetch_path: &MergePath,
    requiring_fetch_step_index: Option<NodeIndex>,
    created_from_requires: bool,
) -> Result<Vec<NodeIndex>, FetchGraphError> {
    // We're at the root
    let fetch_step_index = parent_fetch_step_index
        .unwrap_or_else(|| create_noop_fetch_step(fetch_graph, created_from_requires));

    process_children_for_fetch_steps(
        graph,
        fetch_graph,
        query_node,
        fetch_step_index,
        response_path,
        fetch_path,
        requiring_fetch_step_index,
        created_from_requires,
    )
}

fn add_typename_field_to_output(
    fetch_step: &mut FetchStepData,
    type_name: &str,
    add_at: &MergePath,
) {
    trace!("adding __typename field to output for type '{}'", type_name);

    fetch_step.output.add_at_path(
        &TypeAwareSelection {
            selection_set: SelectionSet {
                items: vec![SelectionItem::Field(FieldSelection::new_typename())],
            },
            type_name: type_name.to_string(),
        },
        add_at.clone(),
        true,
    );
}

// TODO: simplfy args
#[allow(clippy::too_many_arguments)]
#[instrument(level = "trace",skip_all, fields(
  edge = graph.pretty_print_edge(edge_index, false),
  parent_fetch_step_index = parent_fetch_step_index.index(),
  requiring_fetch_step_index = requiring_fetch_step_index.map(|f| f.index()),
))]
fn process_entity_move_edge(
    graph: &Graph,
    fetch_graph: &mut FetchGraph,
    query_node: &QueryTreeNode,
    parent_fetch_step_index: NodeIndex,
    response_path: &MergePath,
    fetch_path: &MergePath,
    requiring_fetch_step_index: Option<NodeIndex>,
    edge_index: EdgeIndex,
    created_from_requires: bool,
) -> Result<Vec<NodeIndex>, FetchGraphError> {
    let edge = graph.edge(edge_index)?;
    let (requirement, is_interface) = match edge {
        Edge::EntityMove(em) => (
            TypeAwareSelection {
                selection_set: em.requirements.selection_set.clone(),
                type_name: em.requirements.type_name.clone(),
            },
            em.is_interface,
        ),
        _ => {
            return Err(FetchGraphError::UnexpectedEdgeMove(
                "EntityMove".to_string(),
            ))
        }
    };

    let head_node_index = graph.get_edge_head(&edge_index)?;
    let head_node = graph.node(head_node_index)?;
    let input_type_name = match head_node {
        Node::SubgraphType(t) => &t.name,
        _ => return Err(FetchGraphError::ExpectedSubgraphType),
    };

    let tail_node_index = graph.get_edge_tail(&edge_index)?;
    let tail_node = graph.node(tail_node_index)?;
    let (output_type_name, subgraph_name) = match tail_node {
        Node::SubgraphType(t) => (&t.name, &t.subgraph),
        _ => return Err(FetchGraphError::ExpectedSubgraphType),
    };

    let fetch_step_index = ensure_fetch_step_for_subgraph(
        fetch_graph,
        parent_fetch_step_index,
        subgraph_name,
        input_type_name,
        output_type_name,
        response_path,
        Some(&requirement),
        None,
        created_from_requires,
    )?;

    let fetch_step = fetch_graph.get_step_data_mut(fetch_step_index)?;
    trace!(
        "adding input requirement '{}' to fetch step [{}]",
        requirement,
        fetch_step_index.index()
    );
    fetch_step.input.add(&requirement);

    if is_interface {
        // We use `output_type_name` as there's no connection from `Interface` to `Object`,
        // it's always Object -> Interface.
        trace!(
            "adding input rewrite '... on {} {{ __typename }}' to '{}'",
            output_type_name,
            output_type_name
        );
        fetch_step.add_input_rewrite(FetchRewrite::ValueSetter(ValueSetter {
            path: vec![
                format!("... on {}", output_type_name),
                "__typename".to_string(),
            ],
            set_value_to: output_type_name.clone().into(),
        }));
    }

    let parent_fetch_step = fetch_graph.get_step_data_mut(parent_fetch_step_index)?;
    add_typename_field_to_output(parent_fetch_step, output_type_name, fetch_path);

    // Make the fetch step a child of the parent fetch step
    trace!(
        "connecting fetch step to parent [{}] -> [{}]",
        parent_fetch_step_index.index(),
        fetch_step_index.index()
    );
    fetch_graph.connect(parent_fetch_step_index, fetch_step_index);

    process_requirements_for_fetch_steps(
        graph,
        fetch_graph,
        query_node,
        parent_fetch_step_index,
        Some(fetch_step_index),
        response_path,
        fetch_path,
    )?;

    process_children_for_fetch_steps(
        graph,
        fetch_graph,
        query_node,
        fetch_step_index,
        response_path,
        &MergePath::default(),
        requiring_fetch_step_index,
        created_from_requires,
    )
}

// TODO: simplfy args
#[allow(clippy::too_many_arguments)]
#[instrument(level = "trace",skip_all, fields(
  edge = graph.pretty_print_edge(edge_index, false),
  parent_fetch_step_index = parent_fetch_step_index.index(),
  requiring_fetch_step_index = requiring_fetch_step_index.map(|f| f.index()),
))]
fn process_interface_object_type_move_edge(
    graph: &Graph,
    fetch_graph: &mut FetchGraph,
    query_node: &QueryTreeNode,
    parent_fetch_step_index: NodeIndex,
    response_path: &MergePath,
    fetch_path: &MergePath,
    requiring_fetch_step_index: Option<NodeIndex>,
    edge_index: EdgeIndex,
    object_type_name: &str,
    created_from_requires: bool,
) -> Result<Vec<NodeIndex>, FetchGraphError> {
    let edge = graph.edge(edge_index)?;
    let requirement = match edge {
        Edge::InterfaceObjectTypeMove(m) => TypeAwareSelection {
            selection_set: m.requirements.selection_set.clone(),
            type_name: m.requirements.type_name.clone(),
        },
        _ => {
            return Err(FetchGraphError::UnexpectedEdgeMove(
                "InterfaceObjectTypeMove".to_string(),
            ))
        }
    };

    let tail_node_index = graph.get_edge_tail(&edge_index)?;
    let tail_node = graph.node(tail_node_index)?;
    let (interface_type_name, subgraph_name) = match tail_node {
        Node::SubgraphType(t) => (&t.name, &t.subgraph),
        // todo: FetchGraphError::MissingSubgraphName(tail_node.clone())
        _ => return Err(FetchGraphError::ExpectedSubgraphType),
    };

    let fetch_step_index = ensure_fetch_step_for_subgraph(
        fetch_graph,
        parent_fetch_step_index,
        subgraph_name,
        object_type_name,
        interface_type_name,
        response_path,
        Some(&requirement),
        None,
        created_from_requires,
    )?;

    let fetch_step = fetch_graph.get_step_data_mut(fetch_step_index)?;
    trace!(
        "adding input requirement '{}' to fetch step [{}]",
        requirement,
        fetch_step_index.index()
    );
    fetch_step.input.add(&requirement);
    let key_to_reenter_subgraph =
        find_satisfiable_key(graph, query_node.requirements.first().unwrap())?;
    fetch_step.input.add(&requirement);
    trace!(
        "adding key '{}' to fetch step [{}]",
        key_to_reenter_subgraph,
        fetch_step_index.index()
    );
    fetch_step.input.add(key_to_reenter_subgraph);

    trace!(
        "adding input rewrite '... on {} {{ __typename }}' to '{}'",
        interface_type_name,
        interface_type_name
    );
    fetch_step.add_input_rewrite(FetchRewrite::ValueSetter(ValueSetter {
        path: vec![
            format!("... on {}", interface_type_name),
            "__typename".to_string(),
        ],
        set_value_to: interface_type_name.clone().into(),
    }));

    let parent_fetch_step = fetch_graph.get_step_data_mut(parent_fetch_step_index)?;
    add_typename_field_to_output(parent_fetch_step, interface_type_name, fetch_path);

    // Make the fetch step a child of the parent fetch step
    trace!(
        "connecting fetch step to parent [{}] -> [{}]",
        parent_fetch_step_index.index(),
        fetch_step_index.index()
    );
    fetch_graph.connect(parent_fetch_step_index, fetch_step_index);

    process_requirements_for_fetch_steps(
        graph,
        fetch_graph,
        query_node,
        parent_fetch_step_index,
        Some(fetch_step_index),
        response_path,
        fetch_path,
    )?;

    process_children_for_fetch_steps(
        graph,
        fetch_graph,
        query_node,
        fetch_step_index,
        response_path,
        &MergePath::default(),
        requiring_fetch_step_index,
        created_from_requires,
    )
}

#[instrument(level = "trace",skip_all, fields(
  subgraph = subgraph_name.0,
  type_name = type_name,
  parent_fetch_step_index = parent_fetch_step_index.index(),
))]
fn process_subgraph_entrypoint_edge(
    graph: &Graph,
    fetch_graph: &mut FetchGraph,
    query_node: &QueryTreeNode,
    parent_fetch_step_index: NodeIndex,
    subgraph_name: &SubgraphName,
    type_name: &str,
    created_from_requires: bool,
) -> Result<Vec<NodeIndex>, FetchGraphError> {
    let fetch_step_index = create_fetch_step_for_root_move(
        fetch_graph,
        parent_fetch_step_index,
        subgraph_name,
        type_name,
        query_node.mutation_field_position,
    );

    fetch_graph.connect(parent_fetch_step_index, fetch_step_index);

    process_children_for_fetch_steps(
        graph,
        fetch_graph,
        query_node,
        fetch_step_index,
        &MergePath::default(),
        &MergePath::default(),
        None,
        created_from_requires,
    )
}

// TODO: simplfy args
#[allow(clippy::too_many_arguments)]
#[instrument(level = "trace",skip_all, fields(
  parent_fetch_step_index = parent_fetch_step_index.index(),
  requiring_fetch_step_index = requiring_fetch_step_index.map(|f| f.index()),
  type_name = target_type_name,
  response_path = response_path.to_string(),
  fetch_path = fetch_path.to_string()
))]
fn process_abstract_edge(
    graph: &Graph,
    fetch_graph: &mut FetchGraph,
    query_node: &QueryTreeNode,
    parent_fetch_step_index: NodeIndex,
    requiring_fetch_step_index: Option<NodeIndex>,
    response_path: &MergePath,
    fetch_path: &MergePath,
    target_type_name: &String,
    edge_index: &EdgeIndex,
) -> Result<Vec<NodeIndex>, FetchGraphError> {
    let head_index = graph.get_edge_head(edge_index)?;
    let head = graph.node(head_index)?;
    let head_type_name = match head {
        Node::SubgraphType(t) => &t.name,
        _ => return Err(FetchGraphError::ExpectedSubgraphType),
    };

    let parent_fetch_step = fetch_graph.get_step_data_mut(parent_fetch_step_index)?;
    trace!(
        "adding output field '__typename' and starting an inline fragment for type '{}' to fetch step [{}]",
        parent_fetch_step_index.index(),
        target_type_name,
    );
    parent_fetch_step.output.add_at_path(
        &TypeAwareSelection {
            selection_set: SelectionSet {
                items: vec![
                    SelectionItem::Field(FieldSelection::new_typename()),
                    SelectionItem::InlineFragment(InlineFragmentSelection {
                        type_condition: target_type_name.clone(),
                        selections: SelectionSet::default(),
                    }),
                ],
            },
            type_name: head_type_name.clone(),
        },
        fetch_path.clone(),
        false,
    );

    let child_response_path = response_path.push(Segment::Cast(target_type_name.clone()));
    let child_fetch_path = fetch_path.push(Segment::Cast(target_type_name.clone()));

    process_children_for_fetch_steps(
        graph,
        fetch_graph,
        query_node,
        parent_fetch_step_index,
        &child_response_path,
        &child_fetch_path,
        requiring_fetch_step_index,
        false,
    )
}

// TODO: simplfy args
#[allow(clippy::too_many_arguments)]
#[instrument(level = "trace",skip_all, fields(
  parent_fetch_step_index = parent_fetch_step_index.index(),
  requiring_fetch_step_index = requiring_fetch_step_index.map(|f| f.index()),
  type_name = field_move.type_name,
  field = field_move.name,
  alias = query_node.selection_attributes.as_ref().and_then(|v| v.alias.as_ref()),
  arguments = query_node.selection_attributes.as_ref().and_then(|v| v.arguments.as_ref()).map(|v| format!("{}", v)),
  leaf = field_move.is_leaf,
  list = field_move.is_list,
  response_path = response_path.to_string(),
  fetch_path = fetch_path.to_string()
))]
fn process_plain_field_edge(
    graph: &Graph,
    fetch_graph: &mut FetchGraph,
    query_node: &QueryTreeNode,
    parent_fetch_step_index: NodeIndex,
    requiring_fetch_step_index: Option<NodeIndex>,
    response_path: &MergePath,
    fetch_path: &MergePath,
    field_move: &FieldMove,
    created_from_requires: bool,
) -> Result<Vec<NodeIndex>, FetchGraphError> {
    if let Some(requiring_fetch_step_index) = requiring_fetch_step_index {
        trace!(
            "connecting parent fetch step [{}] to requiring fetch step [{}]",
            parent_fetch_step_index.index(),
            requiring_fetch_step_index.index()
        );
        fetch_graph.connect(parent_fetch_step_index, requiring_fetch_step_index);
    }

    let parent_fetch_step = fetch_graph.get_step_data_mut(parent_fetch_step_index)?;
    trace!(
        "adding output field '{}' to fetch step [{}]",
        field_move.name,
        parent_fetch_step_index.index()
    );

    parent_fetch_step.output.add_at_path(
        &TypeAwareSelection {
            selection_set: SelectionSet {
                items: vec![SelectionItem::Field(FieldSelection {
                    name: field_move.name.to_string(),
                    alias: query_node.selection_alias().map(|a| a.to_string()),
                    selections: SelectionSet::default(),
                    arguments: query_node.selection_arguments().cloned(),
                    skip_if: None,
                    include_if: None,
                })],
            },
            type_name: field_move.type_name.to_string(),
        },
        fetch_path.clone(),
        false,
    );

    let child_segment = query_node.selection_alias().unwrap_or(&field_move.name);
    let segment_args_hash = query_node
        .selection_arguments()
        .map(|a| a.hash_u64())
        .unwrap_or(0);
    let mut child_response_path =
        response_path.push(Segment::Field(child_segment.to_string(), segment_args_hash));
    let mut child_fetch_path =
        fetch_path.push(Segment::Field(child_segment.to_string(), segment_args_hash));

    if field_move.is_list {
        child_response_path = child_response_path.push(Segment::List);
        child_fetch_path = child_fetch_path.push(Segment::List);
    }

    process_children_for_fetch_steps(
        graph,
        fetch_graph,
        query_node,
        parent_fetch_step_index,
        &child_response_path,
        &child_fetch_path,
        requiring_fetch_step_index,
        created_from_requires,
    )
}

// todo: simplify args
#[allow(clippy::too_many_arguments)]
#[instrument(level = "trace",skip_all, fields(
  parent_fetch_step_index = parent_fetch_step_index.index(),
  requiring_fetch_step_index = requiring_fetch_step_index.map(|s| s.index()),
))]
fn process_requires_field_edge(
    graph: &Graph,
    fetch_graph: &mut FetchGraph,
    query_node: &QueryTreeNode,
    parent_fetch_step_index: NodeIndex,
    response_path: &MergePath,
    requiring_fetch_step_index: Option<NodeIndex>,
    field_move: &FieldMove,
    edge_index: EdgeIndex,
    created_from_requires: bool,
) -> Result<Vec<NodeIndex>, FetchGraphError> {
    if fetch_graph.parents_of(parent_fetch_step_index).count() != 1 {
        return Err(FetchGraphError::NonSingleParent);
    }

    let parent_parent_index = fetch_graph
        .parents_of(parent_fetch_step_index)
        .next()
        .map(|edge| edge.source())
        .unwrap();

    let requires = field_move
        .requirements
        .as_ref()
        .ok_or(FetchGraphError::MissingRequires)?;

    let tail_node_index = graph.get_edge_tail(&edge_index)?;
    let tail_node = graph.node(tail_node_index)?;
    let tail_type_name = match tail_node {
        Node::SubgraphType(t) => &t.name,
        _ => return Err(FetchGraphError::ExpectedSubgraphType),
    };
    let head_node_index = graph.get_edge_head(&edge_index)?;
    let head_node = graph.node(head_node_index)?;
    let (head_type_name, head_subgraph_name) = match head_node {
        Node::SubgraphType(t) => (&t.name, &t.subgraph),
        _ => return Err(FetchGraphError::ExpectedSubgraphType),
    };

    let key_to_reenter_subgraph =
        find_satisfiable_key(graph, query_node.requirements.first().unwrap())?;
    trace!("Key to re-enter: {}", key_to_reenter_subgraph);

    let parent_fetch_step = fetch_graph.get_step_data(parent_fetch_step_index)?;
    // In case of a field with `@requires`, the parent will be the current subgraph we're in.
    let real_parent_fetch_step_index = match parent_fetch_step.output.type_name != *head_type_name {
        // If the parent's output resolves a different type, then it's a root type.
        // We can use that as a parent.
        true => parent_fetch_step_index,
        // If the parent's output resolves the same type, it manes we're in an entity call.
        // We need to move up, as the entity call was created to fetch regular fields of the type
        // (those without @requires).
        //
        // Example: Fetch Step was created to get `baz`
        // {
        //   foo
        //   bar @requires(fields: "foo")
        //   baz
        // }
        //
        // We need to stick to the parent of the parent.
        false => parent_parent_index,
    };

    // When a field (foo) is annotated with `@requires(fields: "bar")`
    // We want to create new FetchStep (entity move) for that field (foo)
    // or reuse an existing one if the requirement matches
    // The new FetchStep will have `foo` as the output and `bar` as the input.
    // The `bar` should be fetched by one of the parents.
    trace!("Creating a fetch step for children of @requires");
    let step_for_children_index = ensure_fetch_step_for_requirement(
        fetch_graph,
        real_parent_fetch_step_index,
        head_subgraph_name,
        head_type_name,
        response_path,
        requires,
    )?;

    let step_for_children = fetch_graph.get_step_data_mut(step_for_children_index)?;

    step_for_children.output.add_at_path(
        &TypeAwareSelection {
            selection_set: SelectionSet {
                items: vec![SelectionItem::Field(FieldSelection {
                    name: field_move.name.clone(),
                    alias: None,
                    selections: SelectionSet { items: vec![] },
                    arguments: Default::default(),
                    skip_if: None,
                    include_if: None,
                })],
            },
            type_name: tail_type_name.clone(),
        },
        MergePath::default(),
        false,
    );

    trace!(
        "Adding {} to fetch([{}]).input",
        requires,
        step_for_children_index.index()
    );
    step_for_children.input.add(requires);
    trace!(
        "Adding {} to fetch([{}]).input",
        key_to_reenter_subgraph,
        step_for_children_index.index()
    );
    step_for_children.input.add(key_to_reenter_subgraph);

    trace!("Creating a fetch step for requirement of @requires");
    let step_for_requirements_index = create_fetch_step_for_entity_call(
        fetch_graph,
        head_subgraph_name,
        head_type_name,
        head_type_name,
        response_path,
        true,
    );
    let step_for_requirements = fetch_graph.get_step_data_mut(step_for_requirements_index)?;
    trace!(
        "Adding {} to fetch([{}]).input",
        key_to_reenter_subgraph,
        step_for_requirements_index.index()
    );
    step_for_requirements.input.add(key_to_reenter_subgraph);

    let real_parent_fetch_step = fetch_graph.get_step_data_mut(real_parent_fetch_step_index)?;
    real_parent_fetch_step.output.add_at_path(
        key_to_reenter_subgraph,
        response_path.clone(),
        false,
    );

    fetch_graph.connect(real_parent_fetch_step_index, step_for_requirements_index);

    let mut child_response_path = response_path.push(Segment::Field(field_move.name.clone(), 0));
    let mut child_fetch_path =
        MergePath::default().push(Segment::Field(field_move.name.clone(), 0));

    if field_move.is_list {
        child_response_path = child_response_path.push(Segment::List);
        child_fetch_path = child_fetch_path.push(Segment::List);
    }

    trace!("Processing requirements");
    let leaf_fetch_step_indexes = process_query_node(
        graph,
        fetch_graph,
        query_node.requirements.first().unwrap(),
        Some(step_for_requirements_index),
        response_path,
        &MergePath::default(),
        None,
        true,
    )?;

    //
    // Given `f0 { f1 @requires(fields: f2) }`
    // - step_for_requirements    -> f2
    // - step_for_children        -> f1
    // - parent                   -> f0
    //
    // parent -> step_for_requirements -> step_for_children
    //
    // f0 -> f2 -> f1
    //
    // in case of `f2 @requires(fields: f3)`:
    // f0 -> f3 -> f2 -> f1
    //
    // and so on.
    //
    // Basically any leaf becomes a parent of current `step_for_children`.
    // This way we wait for the entire chain of fetches to be resolved before we move to resolve a field with `@requires`.

    if leaf_fetch_step_indexes.is_empty() {
        trace!("Connecting fetch that pulls requirements with fetch that resolves the field with @requires");
        fetch_graph.connect(step_for_requirements_index, step_for_children_index);
    } else {
        trace!("Connecting leaf fetches of requirements to fetch with @requires");
        for idx in leaf_fetch_step_indexes {
            fetch_graph.connect(idx, step_for_children_index);
        }
    }

    trace!("Processing children");
    process_children_for_fetch_steps(
        graph,
        fetch_graph,
        query_node,
        step_for_children_index,
        &child_response_path,
        &child_fetch_path,
        requiring_fetch_step_index,
        created_from_requires,
    )
}

/// A field marked with `@requires` tells us that in order to resolve it,
/// the subgraph needs data from another subgraph.
///
/// This interaction ALWAYS involves an entity call,
/// but the trigger for resolving the field differs:
///
/// 1.  Cross-Subgraph: If the field annotated with `@requires` is being
///     fetched after an `EntityMove` brought us to the current subgraph from another,
///     the `EntityMove` itself already handled fetching the necessary `@key` fields.
///     We don't need to think about it, we just need to add the field set of `@requires(fields:)`
///     to the `FetchStep` input.
///
/// 2.  Root: If the field annotated with `@requires` is being fetched locally,
///     because the query started from the root type,
///     the `EntityMove` is needed, but the data to satisfy the key fields is all local.
///     The subgraph effectively makes an "internal" entity call to itself.
///     - We first fetch the fields needed to satisfy some resolvable `@key` of the
///       entity type within this subgraph.
///     - Then, we use that to resolve the fields specified in the `@requires(fields:)`.
///     - This effectively splits the resolution within the subgraph: part comes from the
///       main query path, part comes via the internal entity resolution triggered by `@requires`.
///
/// The key fields need to be added to the output of the parent,
/// and input of the entity move.
#[instrument(level = "trace",skip_all, fields(
  node = graph.node(query_node.node_index).unwrap().to_string()
))]
fn find_satisfiable_key<'a>(
    graph: &'a Graph,
    query_node: &QueryTreeNode,
) -> Result<&'a TypeAwareSelection, FetchGraphError> {
    // This could be improved...
    // We added a flag to `can_satisfy_edge` and increased the complexity.

    let mut entity_moves_edges_to_self: Vec<EdgeReference<crate::graph::edge::Edge>> = graph
        .edges_from(query_node.node_index)
        .filter(|edge_reference| {
            let edge = graph.edge(edge_reference.id()).unwrap();
            matches!(edge, Edge::EntityMove(_))
        })
        .collect();
    entity_moves_edges_to_self.sort_by_key(|edge| std::cmp::Reverse(edge.weight().cost()));

    for edge_ref in entity_moves_edges_to_self {
        if can_satisfy_edge(
            graph,
            &edge_ref,
            &OperationPath {
                root_node: query_node.node_index,
                last_segment: None,
                visited_edge_indices: Default::default(),
                cost: 0,
            },
            &Default::default(),
            true,
        )
        .map_err(|err| FetchGraphError::SatisfiableKeyFailure(Box::new(err)))?
        .is_some()
        {
            return edge_ref
                .weight()
                .requirements()
                .ok_or(FetchGraphError::Internal(String::from(
                    "Resolved empty Satisfiable Key",
                )));
        }
    }

    Err(FetchGraphError::Internal(String::from(
        "Failed to find Satisfiable Key",
    )))
}

// TODO: simplfy args
#[allow(clippy::too_many_arguments)]
fn process_query_node(
    graph: &Graph,
    fetch_graph: &mut FetchGraph,
    query_node: &QueryTreeNode,
    parent_fetch_step_index: Option<NodeIndex>,
    response_path: &MergePath,
    fetch_path: &MergePath,
    requiring_fetch_step_index: Option<NodeIndex>,
    created_from_requires: bool,
) -> Result<Vec<NodeIndex>, FetchGraphError> {
    if let Some(edge_index) = query_node.edge_from_parent {
        let parent_fetch_step_index = parent_fetch_step_index.ok_or(FetchGraphError::IndexNone)?;
        let edge = graph.edge(edge_index)?;

        match edge {
            Edge::SubgraphEntrypoint { name, .. } => {
                let tail_node_index = graph.get_edge_tail(&edge_index)?;
                let tail_node = graph.node(tail_node_index)?;
                let type_name = match tail_node {
                    Node::QueryRoot(t) => t,
                    Node::MutationRoot(t) => t,
                    Node::SubscriptionRoot(t) => t,
                    Node::SubgraphType(t) => &t.name,
                };

                process_subgraph_entrypoint_edge(
                    graph,
                    fetch_graph,
                    query_node,
                    parent_fetch_step_index,
                    name,
                    type_name,
                    created_from_requires,
                )
            }
            Edge::EntityMove(_) => process_entity_move_edge(
                graph,
                fetch_graph,
                query_node,
                parent_fetch_step_index,
                response_path,
                fetch_path,
                requiring_fetch_step_index,
                edge_index,
                created_from_requires,
            ),
            Edge::FieldMove(field) => match field.requirements.is_some() {
                true => process_requires_field_edge(
                    graph,
                    fetch_graph,
                    query_node,
                    parent_fetch_step_index,
                    response_path,
                    requiring_fetch_step_index,
                    field,
                    edge_index,
                    created_from_requires,
                ),
                false => process_plain_field_edge(
                    graph,
                    fetch_graph,
                    query_node,
                    parent_fetch_step_index,
                    requiring_fetch_step_index,
                    response_path,
                    fetch_path,
                    field,
                    created_from_requires,
                ),
            },
            Edge::AbstractMove(type_name) => process_abstract_edge(
                graph,
                fetch_graph,
                query_node,
                parent_fetch_step_index,
                requiring_fetch_step_index,
                response_path,
                fetch_path,
                type_name,
                &edge_index,
            ),
            Edge::InterfaceObjectTypeMove(InterfaceObjectTypeMove {
                object_type_name, ..
            }) => process_interface_object_type_move_edge(
                graph,
                fetch_graph,
                query_node,
                parent_fetch_step_index,
                response_path,
                fetch_path,
                requiring_fetch_step_index,
                edge_index,
                object_type_name,
                created_from_requires,
            ),
        }
    } else {
        process_noop_edge(
            graph,
            fetch_graph,
            query_node,
            parent_fetch_step_index,
            response_path,
            fetch_path,
            requiring_fetch_step_index,
            created_from_requires,
        )
    }
}

pub fn find_graph_roots(graph: &FetchGraph) -> Vec<NodeIndex> {
    let mut roots = Vec::new();

    // Iterate over all nodes in the graph
    for node_idx in graph.step_indices() {
        // Check if the node has any incoming edges.
        // The `next().is_none()` checks if the iterator is empty - no incoming edges.
        if graph.parents_of(node_idx).next().is_none() {
            roots.push(node_idx);
        }
    }

    roots
}

#[instrument(level = "trace", skip(graph, query_tree), fields(
    requirements_count = query_tree.root.requirements.len(),
    children_count = query_tree.root.children.len(),
))]
pub fn build_fetch_graph_from_query_tree(
    graph: &Graph,
    query_tree: QueryTree,
) -> Result<FetchGraph, FetchGraphError> {
    let mut fetch_graph = FetchGraph::new();

    process_query_node(
        graph,
        &mut fetch_graph,
        &query_tree.root,
        None,
        &MergePath::default(),
        &MergePath::default(),
        None,
        false,
    )?;

    trace!("Done");

    let root_indexes = find_graph_roots(&fetch_graph);

    trace!("found roots");

    if root_indexes.is_empty() {
        return Err(FetchGraphError::NonSingleRootStep(0));
    }

    if root_indexes.len() > 1 {
        return Err(FetchGraphError::NonSingleRootStep(root_indexes.len()));
    }

    trace!("fetch graph before optimizations:");
    trace!("{}", fetch_graph);

    // fine to unwrap as we have already checked the length
    fetch_graph.root_index = Some(*root_indexes.first().unwrap());
    fetch_graph.optimize()?;
    fetch_graph.collect_variable_usages()?;

    trace!("fetch graph after optimizations:");
    trace!("{}", fetch_graph);

    Ok(fetch_graph)
}

fn is_mutation_fetch_step(
    fetch_graph: &mut FetchGraph,
    fetch_step_index: NodeIndex,
) -> Result<bool, FetchGraphError> {
    for edge_ref in fetch_graph.children_of(fetch_step_index) {
        let child = fetch_graph.get_step_data(edge_ref.target().id())?;

        if child.output.type_name.ne("Mutation") {
            return Ok(false);
        }
    }

    Ok(true)
}

/// Checks if an ancestor node (`target_ancestor_index`) is reachable from a
/// child node (`child_index`) in a directed graph by following paths upwards
/// (traversing incoming edges), EXCLUDING any paths that start by traversing
/// the direct edge from the `target_ancestor_index` down to the `child_index`.
///
/// This is implemented as an iterative Breadth-First Search (BFS).
/// The search starts from all direct parents of `child_index` *except*
/// `target_ancestor_index`, and follows incoming edges from there.
pub fn is_reachable_via_alternative_upstream_path(
    graph: &FetchGraph,
    child_index: NodeIndex,
    target_ancestor_index: NodeIndex,
) -> Result<bool, FetchGraphError> {
    let mut queue: VecDeque<NodeIndex> = VecDeque::new();
    let mut visited: HashSet<NodeIndex> = HashSet::new();

    // Start BFS queue with all parents of `child_index` except `target_ancestor_index`
    for edge_ref in graph.parents_of(child_index) {
        let parent_index = edge_ref.source();

        if parent_index != target_ancestor_index {
            queue.push_back(parent_index);
            visited.insert(parent_index);
        }
    }

    if queue.is_empty() {
        return Ok(false);
    }

    // Perform BFS upwards (following incoming edges)
    while let Some(current_index) = queue.pop_front() {
        // If we reached the target ancestor indirectly
        if current_index == target_ancestor_index {
            return Ok(true);
        }

        // Explore further up the graph via the parents of the current node
        for edge_ref in graph.parents_of(current_index) {
            let parent_of_current_index = edge_ref.source();

            if visited.insert(parent_of_current_index) {
                queue.push_back(parent_of_current_index);
            }
        }
    }

    // no indirect path exists
    Ok(false)
}

/// Find the arguments conflicts between two selections.
/// Returns a vector of tuples containing the indices of conflicting fields in both "source" and "other"
/// Both indices are returned in order to allow for easy resolution of conflicts later, in either side.
pub fn find_arguments_conflicts(
    source: &TypeAwareSelection,
    other: &TypeAwareSelection,
) -> Vec<(usize, usize)> {
    other
        .selection_set
        .items
        .iter()
        .enumerate()
        .filter_map(|(index, other_selection)| {
            if let SelectionItem::Field(other_field) = other_selection {
                let other_identifier = other_field.selection_identifier();
                let other_args_hash = other_field.arguments_hash();

                let existing_in_self = source.selection_set.items.iter().enumerate().find_map(
                    |(self_index, self_selection)| {
                        if let SelectionItem::Field(self_field) = self_selection {
                            // If the field selection identifier matches and the arguments hash is different,
                            // then it means that we can't merge the two input siblings
                            if self_field.selection_identifier() == other_identifier
                                && self_field.arguments_hash() != other_args_hash
                            {
                                return Some(self_index);
                            }
                        }

                        None
                    },
                );

                if let Some(existing_index) = existing_in_self {
                    return Some((existing_index, index));
                }

                return None;
            }

            None
        })
        .collect()
}

pub struct CanMergeSiblingsResult {
    /// vector of conflicting input indices: (index_in_current, index_in_other)
    pub conflicting_input_fields: Vec<(usize, usize)>,
    /// vector of conflicting input indices: (index_in_current, index_in_other)
    pub conflicting_output_fields: Vec<(usize, usize)>,
}

impl CanMergeSiblingsResult {
    pub fn has_input_conflicts(&self) -> bool {
        !self.conflicting_input_fields.is_empty()
    }

    pub fn has_output_conflicts(&self) -> bool {
        !self.conflicting_output_fields.is_empty()
    }
}
