use crate::ast::merge_path::MergePath;
use crate::ast::selection_item::SelectionItem;
use crate::ast::selection_set::{FieldSelection, SelectionSet};
use crate::ast::type_aware_selection::TypeAwareSelection;
use crate::graph::edge::{Edge, FieldMove};
use crate::graph::node::Node;
use crate::graph::Graph;
use crate::planner::tree::query_tree::QueryTree;
use crate::planner::tree::query_tree_node::QueryTreeNode;
use crate::planner::walker::path::OperationPath;
use crate::planner::walker::pathfinder::can_satisfy_edge;
use crate::state::supergraph_state::SubgraphName;
use petgraph::graph::EdgeReference;
use petgraph::stable_graph::{EdgeIndex, NodeIndex, NodeIndices, StableDiGraph};
use petgraph::visit::Bfs;
use petgraph::visit::{EdgeRef, NodeRef};
use petgraph::Directed;
use petgraph::Direction;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt::{Debug, Display};
use tracing::{debug, instrument};

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
}

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

    #[instrument(skip_all, fields(
      parent = parent_index.index(),
      child = child_index.index(),
    ))]
    pub fn connect(&mut self, parent_index: NodeIndex, child_index: NodeIndex) -> EdgeIndex {
        self.graph.update_edge(parent_index, child_index, ())
    }

    pub fn remove_edge(&mut self, edge_index: EdgeIndex) -> bool {
        self.graph.remove_edge(edge_index).is_some_and(|_| true)
    }

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

    #[instrument(skip_all)]
    pub fn optimize(&mut self) -> Result<(), FetchGraphError> {
        self.merge_children_with_parents()?;
        self.merge_siblings()?;
        self.deduplicate_and_prune_fetch_steps()?;
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
    #[instrument(skip_all)]
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

                debug!("optimization found: remove '{}'", step);

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

    #[instrument(skip_all)]
    fn merge_siblings(&mut self) -> Result<(), FetchGraphError> {
        let root_index = self
            .root_index
            .ok_or(FetchGraphError::NonSingleRootStep(0))?;
        // Breadth-First Search (BFS) starting from the root node.
        let mut queue = VecDeque::from([root_index]);
        while let Some(parent_index) = queue.pop_front() {
            // Store pairs of sibling nodes that can be merged.
            let mut merges_to_perform: Vec<(NodeIndex, NodeIndex)> = Vec::new();
            // HashMap to keep track of node index mappings, especially after merges.
            // Key: original index, Value: potentially updated index after merges.
            let mut node_indexes: HashMap<NodeIndex, NodeIndex> = HashMap::new();
            let children: Vec<_> = self
                .graph
                .neighbors_directed(parent_index, Direction::Outgoing)
                .collect();

            for (i, child_index) in children.iter().enumerate() {
                // Add the current child to the queue for further processing (BFS).
                queue.push_back(*child_index);
                let child = self.get_step_data(*child_index)?;

                // Iterate through the remaining children (siblings) to check for merge possibilities.
                for other_child_index in children.iter().skip(i + 1) {
                    let other_child = self.get_step_data(*other_child_index)?;

                    if child.can_merge_sibling(*child_index, *other_child_index, other_child, self)
                    {
                        debug!(
                            "Found optimization: {} <- {}",
                            child_index.index(),
                            other_child_index.index()
                        );
                        // Register their original indexes in the map.
                        node_indexes.insert(*child_index, *child_index);
                        node_indexes.insert(*other_child_index, *other_child_index);
                        merges_to_perform.push((*child_index, *other_child_index));
                        // Since a merge is possible, move to the next child to avoid redundant checks.
                        break;
                    }
                }
            }

            for (child_index, other_child_index) in merges_to_perform {
                // Get the latest indexes for the nodes, accounting for previous merges.
                let child_index_latest = node_indexes
                    .get(&child_index)
                    .expect("Index mapping got lost");
                let other_child_index_latest = node_indexes
                    .get(&other_child_index)
                    .expect("Index mapping got lost");

                perform_fetch_step_sibling_merge(
                    *child_index_latest,
                    *other_child_index_latest,
                    self,
                )?;

                // Because `other_child` was merged into `child`,
                // then everything that was pointing to `other_child`
                // has to point to the `child`.
                node_indexes.insert(*other_child_index_latest, *child_index_latest);
            }
        }

        Ok(())
    }

    #[instrument(skip_all)]
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

                if parent.can_merge_child(parent_index, *child_index, child, self) {
                    debug!(
                        "optimization found: merge parent '{}' with child '{}'",
                        parent, child
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

                perform_fetch_step_child_merge(*parent_index_latest, *child_index_latest, self)?;

                // Because `child` was merged into `parent`,
                // then everything that was pointing to `child`
                // has to point to the `parent`.
                node_indexes.insert(*child_index_latest, *parent_index_latest);
            }
        }

        Ok(())
    }

    #[instrument(skip_all)]
    fn turn_mutations_into_sequence(&mut self) -> Result<(), FetchGraphError> {
        let root_index = self
            .root_index
            .ok_or(FetchGraphError::NonSingleRootStep(0))?;
        if !is_mutation_fetch_step(self, root_index)? {
            return Ok(());
        }

        let mut new_edges_pairs: Vec<(NodeIndex, NodeIndex)> = Vec::new();
        let mut edge_ids_to_remove: Vec<EdgeIndex> = Vec::new();
        let mut iter = self.children_of(root_index);
        let mut current = iter.next();

        for next_edge in iter {
            if let Some(curr_edge) = current {
                let current_node_index = curr_edge.target().id();
                let next_node_index = next_edge.target().id();

                // no need to remove the initial edge (root -> Mutation)
                edge_ids_to_remove.push(next_edge.id());
                new_edges_pairs.push((current_node_index, next_node_index));
            }
            current = Some(next_edge);
        }

        for (from_id, to_id) in new_edges_pairs {
            self.connect(from_id, to_id);
        }

        for edge_id in edge_ids_to_remove {
            self.remove_edge(edge_id);
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
            let fetch_step = self.graph.node_weight(node_index).expect("node to exist");

            fetch_step.pretty_write(f, node_index)?;
            writeln!(f)?;
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

#[derive(Debug, Clone)]
pub struct FetchStepData {
    pub service_name: SubgraphName,
    pub response_path: MergePath,
    pub input: TypeAwareSelection,
    pub output: TypeAwareSelection,
    pub reserved_for_requires: Option<TypeAwareSelection>,
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
        )
    }
}

impl FetchStepData {
    pub fn is_reserved_for_requires(&self, selection: &TypeAwareSelection) -> bool {
        self.reserved_for_requires
            .as_ref()
            .is_some_and(|requires| requires.eq(selection))
    }

    pub fn pretty_write(
        &self,
        writer: &mut std::fmt::Formatter<'_>,
        index: NodeIndex,
    ) -> Result<(), std::fmt::Error> {
        write!(writer, "[{}] {}", index.index(), self)
    }

    pub fn can_merge_child(
        &self,
        self_index: NodeIndex,
        other_index: NodeIndex,
        other: &Self,
        fetch_graph: &FetchGraph,
    ) -> bool {
        if self_index == other_index {
            return false;
        }

        if self.service_name == other.service_name {
            return self.can_merge_sibling(self_index, other_index, other, fetch_graph);
        }

        let self_path = self.response_path.insert_front("*".to_string());
        let other_path = other.response_path.insert_front("*".to_string());
        if !other_path.starts_with(&self_path) {
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

    pub fn can_merge_sibling(
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

        // we should prevent merging steps with more than 1 parent
        // we should merge only if the parent of both is identical

        let self_path = self.response_path.insert_front("*".to_string());
        let other_path = other.response_path.insert_front("*".to_string());
        if !other_path.starts_with(&self_path) {
            return false;
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

#[instrument(skip_all)]
fn perform_fetch_step_sibling_merge(
    self_index: NodeIndex,
    other_index: NodeIndex,
    fetch_graph: &mut FetchGraph,
) -> Result<(), FetchGraphError> {
    let (me, other) = fetch_graph.get_pair_of_steps_mut(self_index, other_index)?;

    debug!("merging fetch steps '{}' and '{}'", me, other);

    me.output.add_at_path(
        &other.output,
        other.response_path.slice_from(me.response_path.len()),
        false,
    );

    if me.input.type_name == other.input.type_name {
        if me.response_path != other.response_path {
            panic!("input types are equal but resonse_path are different, should not happen");
        }

        me.input.add(&other.input);
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
    for child_index in children_indexes {
        fetch_graph.connect(self_index, child_index);
    }
    // 2. Add parent -> self
    for parent_index in parents_indexes {
        fetch_graph.connect(parent_index, self_index);
    }
    // 3. Drop other -> child and parent -> other
    fetch_graph.remove_step(other_index);

    Ok(())
}

#[instrument(skip_all)]
fn perform_fetch_step_child_merge(
    self_index: NodeIndex,
    other_index: NodeIndex,
    fetch_graph: &mut FetchGraph,
) -> Result<(), FetchGraphError> {
    perform_fetch_step_sibling_merge(self_index, other_index, fetch_graph)
}

fn create_noop_fetch_step(fetch_graph: &mut FetchGraph) -> NodeIndex {
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
        reserved_for_requires: None,
    })
}

fn create_fetch_step_for_entity_move(
    fetch_graph: &mut FetchGraph,
    subgraph_name: &SubgraphName,
    type_name: &str,
    response_path: &MergePath,
) -> NodeIndex {
    fetch_graph.add_step(FetchStepData {
        service_name: subgraph_name.clone(),
        response_path: response_path.clone(),
        input: TypeAwareSelection {
            selection_set: SelectionSet {
                items: vec![SelectionItem::Field(FieldSelection::new_typename())],
            },
            type_name: type_name.to_string(),
        },
        output: TypeAwareSelection {
            selection_set: SelectionSet::default(),
            type_name: type_name.to_string(),
        },
        reserved_for_requires: None,
    })
}

fn create_fetch_step_for_root_move(
    fetch_graph: &mut FetchGraph,
    root_step_index: NodeIndex,
    subgraph_name: &SubgraphName,
    type_name: &str,
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
        reserved_for_requires: None,
    });

    fetch_graph.connect(root_step_index, idx);

    idx
}

fn ensure_fetch_step_for_subgraph(
    fetch_graph: &mut FetchGraph,
    parent_fetch_step_index: NodeIndex,
    subgraph_name: &SubgraphName,
    type_name: &String,
    response_path: &MergePath,
    key: Option<&TypeAwareSelection>,
    requires: Option<&TypeAwareSelection>,
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

                    if let Some(key) = &key {
                        if !fetch_step.input.contains(key) {
                            // requested key fields are not part of the input
                            return None;
                        }
                    }

                    // The `requirement` should be equal.
                    // If it's a subset,
                    //  then some other field in step's requirement may postpone/affect the child FetchStep.
                    // If the FetchStep has no requirement,
                    //  then attaching required fields to it may postpone/affect the resolution of other fields.
                    if let Some(requires) = &requires {
                        if !fetch_step.is_reserved_for_requires(requires) {
                            return None;
                        }
                    }

                    return Some(to_child_edge_ref.target());
                }

                None
            });

    match matching_child_index {
        Some(idx) => {
            debug!(
                "found existing fetch step [{}] for entity move requirement({}) key({}) in children of {}",
                idx.index(),
                requires.map(|r| r.to_string()).unwrap_or_default(),
                key.map(|r| r.to_string()).unwrap_or_default(),
                parent_fetch_step_index.index(),
            );
            Ok(idx)
        }
        None => {
            let step_index = create_fetch_step_for_entity_move(
                fetch_graph,
                subgraph_name,
                type_name,
                response_path,
            );
            if let Some(selection) = key {
                let step = fetch_graph.get_step_data_mut(step_index)?;
                step.input.add(selection)
            }

            debug!(
                "created a new fetch step [{}] subgraph({}) type({}) requirement({}) key({}) in children of {}",
                step_index.index(),
                subgraph_name,
                type_name,
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
            debug!(
                "found existing fetch step [{}] children of {}",
                idx.index(),
                parent_fetch_step_index.index(),
            );
            Ok(idx)
        }
        None => {
            let step_index = create_fetch_step_for_entity_move(
                fetch_graph,
                subgraph_name,
                type_name,
                response_path,
            );

            debug!(
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

#[instrument(skip_all, fields(
  count = query_node.children.len(),
  parent_fetch_step_index = parent_fetch_step_index.map(|s| s.index()),
  requiring_fetch_step_index = requiring_fetch_step_index.map(|s| s.index())
))]
fn process_children_for_fetch_steps(
    graph: &Graph,
    fetch_graph: &mut FetchGraph,
    query_node: &QueryTreeNode,
    parent_fetch_step_index: Option<NodeIndex>,
    response_path: &MergePath,
    fetch_path: &MergePath,
    requiring_fetch_step_index: Option<NodeIndex>,
) -> Result<Vec<NodeIndex>, FetchGraphError> {
    if query_node.children.is_empty() {
        return Ok(parent_fetch_step_index.map_or(vec![], |i| vec![i]));
    }

    let mut leaf_fetch_step_indexes: Vec<NodeIndex> = vec![];
    for sub_step in query_node.children.iter() {
        leaf_fetch_step_indexes.extend(process_query_node(
            graph,
            fetch_graph,
            sub_step,
            parent_fetch_step_index,
            response_path,
            fetch_path,
            requiring_fetch_step_index,
        )?);
    }

    Ok(leaf_fetch_step_indexes)
}

#[instrument(skip_all, fields(
  count = query_node.requirements.len()
))]
fn process_requirements_for_fetch_steps(
    graph: &Graph,
    fetch_graph: &mut FetchGraph,
    query_node: &QueryTreeNode,
    parent_fetch_step_index: Option<NodeIndex>,
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
            parent_fetch_step_index,
            response_path,
            fetch_path,
            requiring_fetch_step_index,
        )?;
        fetch_graph.connect(
            parent_fetch_step_index.ok_or(FetchGraphError::IndexNone)?,
            requiring_fetch_step_index.ok_or(FetchGraphError::IndexNone)?,
        );
    }

    Ok(())
}

#[instrument(skip_all)]
fn process_noop_edge(
    graph: &Graph,
    fetch_graph: &mut FetchGraph,
    query_node: &QueryTreeNode,
    parent_fetch_step_index: Option<NodeIndex>,
    response_path: &MergePath,
    fetch_path: &MergePath,
    requiring_fetch_step_index: Option<NodeIndex>,
) -> Result<Vec<NodeIndex>, FetchGraphError> {
    // We're at the root
    let fetch_step_index =
        parent_fetch_step_index.or_else(|| Some(create_noop_fetch_step(fetch_graph)));

    process_children_for_fetch_steps(
        graph,
        fetch_graph,
        query_node,
        fetch_step_index,
        response_path,
        fetch_path,
        requiring_fetch_step_index,
    )
}

fn add_typename_field_to_output(
    fetch_step: &mut FetchStepData,
    type_name: &str,
    add_at: &MergePath,
) {
    debug!("adding __typename field to output for type '{}'", type_name);

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
#[instrument(skip_all, fields(
  edge = graph.pretty_print_edge(edge_index, false),
  parent_fetch_step_index = parent_fetch_step_index.map(|f| f.index()),
  requiring_fetch_step_index = requiring_fetch_step_index.map(|f| f.index()),
))]
fn process_entity_move_edge(
    graph: &Graph,
    fetch_graph: &mut FetchGraph,
    query_node: &QueryTreeNode,
    parent_fetch_step_index: Option<NodeIndex>,
    response_path: &MergePath,
    fetch_path: &MergePath,
    requiring_fetch_step_index: Option<NodeIndex>,
    edge_index: EdgeIndex,
) -> Result<Vec<NodeIndex>, FetchGraphError> {
    if parent_fetch_step_index.is_none() {
        panic!("Expected a parent fetch step");
    }
    let edge = graph.edge(edge_index)?;
    let requirement = match edge {
        Edge::EntityMove(em) => TypeAwareSelection {
            selection_set: em.requirements.selection_set.clone(),
            type_name: em.requirements.type_name.clone(),
        },
        _ => panic!("Expected an entity move"),
    };

    let tail_node_index = graph.get_edge_tail(&edge_index)?;
    let tail_node = graph.node(tail_node_index)?;
    let (type_name, subgraph_name) = match tail_node {
        Node::SubgraphType(t) => (&t.name, &t.subgraph),
        // todo: FetchGraphError::MissingSubgraphName(tail_node.clone())
        _ => panic!("Expected a subgraph type, not root type"),
    };

    let fetch_step_index = ensure_fetch_step_for_subgraph(
        fetch_graph,
        parent_fetch_step_index.ok_or(FetchGraphError::IndexNone)?,
        subgraph_name,
        type_name,
        response_path,
        Some(&requirement),
        None,
    )?;

    let fetch_step = fetch_graph.get_step_data_mut(fetch_step_index)?;
    debug!(
        "adding input requirement '{}' to fetch step [{}]",
        requirement,
        fetch_step_index.index()
    );
    fetch_step.input.add(&requirement);

    let parent_fetch_step = fetch_graph
        .get_step_data_mut(parent_fetch_step_index.ok_or(FetchGraphError::IndexNone)?)?;
    add_typename_field_to_output(parent_fetch_step, type_name, fetch_path);

    // Make the fetch step a child of the parent fetch step
    debug!(
        "connecting fetch step to parent [{}] -> [{}]",
        parent_fetch_step_index
            .ok_or(FetchGraphError::IndexNone)?
            .index(),
        fetch_step_index.index()
    );
    fetch_graph.connect(
        parent_fetch_step_index.ok_or(FetchGraphError::IndexNone)?,
        fetch_step_index,
    );

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
        Some(fetch_step_index),
        response_path,
        &MergePath::default(),
        requiring_fetch_step_index,
    )
}

#[instrument(skip_all, fields(
  subgraph = subgraph_name.0,
  type_name = type_name,
  parent_fetch_step_index = parent_fetch_step_index.map(|f| f.index()),
))]
fn process_subgraph_entrypoint_edge(
    graph: &Graph,
    fetch_graph: &mut FetchGraph,
    query_node: &QueryTreeNode,
    parent_fetch_step_index: Option<NodeIndex>,
    subgraph_name: &SubgraphName,
    type_name: &str,
) -> Result<Vec<NodeIndex>, FetchGraphError> {
    if parent_fetch_step_index.is_none() {
        panic!("Expected a parent fetch step")
    }

    let fetch_step_index = create_fetch_step_for_root_move(
        fetch_graph,
        parent_fetch_step_index.ok_or(FetchGraphError::IndexNone)?,
        subgraph_name,
        type_name,
    );

    fetch_graph.connect(
        parent_fetch_step_index.ok_or(FetchGraphError::IndexNone)?,
        fetch_step_index,
    );

    process_children_for_fetch_steps(
        graph,
        fetch_graph,
        query_node,
        Some(fetch_step_index),
        &MergePath::default(),
        &MergePath::default(),
        None,
    )
}

// TODO: simplfy args
#[allow(clippy::too_many_arguments)]
#[instrument(skip_all, fields(
  parent_fetch_step_index = parent_fetch_step_index.map(|f| f.index()),
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
    parent_fetch_step_index: Option<NodeIndex>,
    requiring_fetch_step_index: Option<NodeIndex>,
    response_path: &MergePath,
    fetch_path: &MergePath,
    field_move: &FieldMove,
) -> Result<Vec<NodeIndex>, FetchGraphError> {
    let parent_fetch_step_index = parent_fetch_step_index.ok_or(FetchGraphError::IndexNone)?;

    if let Some(requiring_fetch_step_index) = requiring_fetch_step_index {
        debug!(
            "connecting parent fetch step [{}] to requiring fetch step [{}]",
            parent_fetch_step_index.index(),
            requiring_fetch_step_index.index()
        );
        fetch_graph.connect(parent_fetch_step_index, requiring_fetch_step_index);
    }

    let parent_fetch_step = fetch_graph.get_step_data_mut(parent_fetch_step_index)?;
    debug!(
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
                })],
            },
            type_name: field_move.type_name.to_string(),
        },
        fetch_path.clone(),
        false,
    );

    let response_path_name = query_node.selection_alias().unwrap_or(&field_move.name);
    let mut child_response_path = response_path.push(response_path_name.to_string());
    let mut child_fetch_path = fetch_path.push(response_path_name.to_string());

    if field_move.is_list {
        child_response_path = child_response_path.push("@".to_string());
        child_fetch_path = child_fetch_path.push("@".to_string());
    }

    process_children_for_fetch_steps(
        graph,
        fetch_graph,
        query_node,
        Some(parent_fetch_step_index),
        &child_response_path,
        &child_fetch_path,
        requiring_fetch_step_index,
    )
}

// todo: simplify args
#[allow(clippy::too_many_arguments)]
#[instrument(skip_all, fields(
  parent_fetch_step_index = parent_fetch_step_index.map(|s| s.index()),
  requiring_fetch_step_index = requiring_fetch_step_index.map(|s| s.index()),
))]
fn process_requires_field_edge(
    graph: &Graph,
    fetch_graph: &mut FetchGraph,
    query_node: &QueryTreeNode,
    parent_fetch_step_index: Option<NodeIndex>,
    response_path: &MergePath,
    requiring_fetch_step_index: Option<NodeIndex>,
    edge_index: EdgeIndex,
) -> Result<Vec<NodeIndex>, FetchGraphError> {
    let parent_fetch_step_index = parent_fetch_step_index.ok_or(FetchGraphError::IndexNone)?;

    if fetch_graph.parents_of(parent_fetch_step_index).count() != 1 {
        return Err(FetchGraphError::NonSingleParent);
    }

    let parent_parent_index = fetch_graph
        .parents_of(parent_fetch_step_index)
        .next()
        .map(|edge| edge.source())
        .unwrap();

    let edge = graph.edge(edge_index)?;
    let (requires, field_name, field_is_list) = match edge {
        Edge::FieldMove(f) => (
            &f.requirements.clone().expect("Expected @requires"),
            &f.name,
            &f.is_list,
        ),
        _ => panic!("Expected a Field Move with @requires"),
    };

    let tail_node_index = graph.get_edge_tail(&edge_index)?;
    let tail_node = graph.node(tail_node_index)?;
    let tail_type_name = match tail_node {
        Node::SubgraphType(t) => &t.name,
        // todo: FetchGraphError::MissingSubgraphName(tail_node.clone())
        _ => panic!("Expected a subgraph type, not root type"),
    };

    let head_node_index = graph.get_edge_head(&edge_index)?;
    let head_node = graph.node(head_node_index)?;
    let (head_type_name, head_subgraph_name) = match head_node {
        Node::SubgraphType(t) => (&t.name, &t.subgraph),
        // todo: FetchGraphError::MissingSubgraphName(head.clone())
        _ => panic!("Expected a subgraph type, not root type"),
    };

    let key_to_reenter_subgraph =
        find_satisfiable_key(graph, query_node.requirements.first().unwrap())?;
    debug!("Key to re-enter: {}", key_to_reenter_subgraph);

    let real_parent_fetch_step_index =
        match fetch_graph.parents_of(parent_parent_index).count() == 0 {
            true => parent_fetch_step_index,
            // In case of a field with `@requires`, the parent will be the current subgraph we're in.
            // That's why we want to move up, to the parent of the current fetch step.
            false => parent_parent_index,
        };

    // When a field (foo) is annotated with `@requires(fields: "bar")`
    // We want to create new FetchStep (entity move) for that field (foo)
    // or reuse an existing one if the requirement matches
    // The new FetchStep will have `foo` as the output and `bar` as the input.
    // The `bar` should be fetched by one of the parents.
    debug!("Creating a fetch step for children of @requires");
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
                    name: field_name.to_owned(),
                    alias: None,
                    selections: SelectionSet { items: vec![] },
                    arguments: Default::default(),
                })],
            },
            type_name: tail_type_name.clone(),
        },
        MergePath::default(),
        false,
    );
    debug!(
        "Adding {} to fetch([{}]).input",
        requires,
        step_for_children_index.index()
    );
    step_for_children.input.add(requires);
    debug!(
        "Adding {} to fetch([{}]).input",
        key_to_reenter_subgraph,
        step_for_children_index.index()
    );
    step_for_children.input.add(key_to_reenter_subgraph);

    debug!("Creating a fetch step for requirement of @requires");
    let step_for_requirements_index = create_fetch_step_for_entity_move(
        fetch_graph,
        head_subgraph_name,
        head_type_name,
        response_path,
    );
    let step_for_requirements = fetch_graph.get_step_data_mut(step_for_requirements_index)?;
    // step_for_requirements.reserved_for_requires = Some(requires.clone());
    debug!(
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

    let mut child_response_path = response_path.push(field_name.to_owned());
    let mut child_fetch_path = MergePath::default().push(field_name.to_owned());

    if *field_is_list {
        child_response_path = child_response_path.push("@".to_string());
        child_fetch_path = child_fetch_path.push("@".to_string());
    }

    debug!("Processing requirements");
    let leaf_fetch_step_indexes = process_query_node(
        graph,
        fetch_graph,
        query_node.requirements.first().unwrap(),
        Some(step_for_requirements_index),
        response_path,
        &MergePath::default(),
        None,
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
        debug!("Connecting fetch that pulls requirements with fetch that resolves the field with @requires");
        fetch_graph.connect(step_for_requirements_index, step_for_children_index);
    } else {
        debug!("Connecting leaf fetches of requirements to fetch with @requires");
        for idx in leaf_fetch_step_indexes {
            fetch_graph.connect(idx, step_for_children_index);
        }
    }

    debug!("Processing children");
    process_children_for_fetch_steps(
        graph,
        fetch_graph,
        query_node,
        Some(step_for_children_index),
        &child_response_path,
        &child_fetch_path,
        requiring_fetch_step_index,
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
#[instrument(skip_all, fields(
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
        .map_err(FetchGraphError::SatisfiableKeyFailure)?
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

fn process_query_node(
    graph: &Graph,
    fetch_graph: &mut FetchGraph,
    query_node: &QueryTreeNode,
    parent_fetch_step_index: Option<NodeIndex>,
    response_path: &MergePath,
    fetch_path: &MergePath,
    requiring_fetch_step_index: Option<NodeIndex>,
) -> Result<Vec<NodeIndex>, FetchGraphError> {
    if let Some(edge_index) = query_node.edge_from_parent {
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
            ),
            Edge::FieldMove(field) => match field.requirements.is_some() {
                true => process_requires_field_edge(
                    graph,
                    fetch_graph,
                    query_node,
                    parent_fetch_step_index,
                    response_path,
                    requiring_fetch_step_index,
                    edge_index,
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
                ),
            },
            Edge::AbstractMove(_) => {
                panic!("AbstractMove is not supported yet")
            }
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

#[instrument(skip(graph, query_tree), fields(
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
    )?;

    debug!("Done");

    let root_indexes = find_graph_roots(&fetch_graph);

    debug!("found roots");

    if root_indexes.is_empty() {
        return Err(FetchGraphError::NonSingleRootStep(0));
    }

    if root_indexes.len() > 1 {
        return Err(FetchGraphError::NonSingleRootStep(root_indexes.len()));
    }

    debug!("print graph");
    debug!("{}", fetch_graph);

    // fine to unwrap as we have already checked the length
    fetch_graph.root_index = Some(*root_indexes.first().unwrap());
    fetch_graph.optimize()?;

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
