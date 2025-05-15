use crate::graph::edge::Edge;
use crate::graph::node::Node;
use crate::graph::Graph;
use crate::planner::tree::query_tree::QueryTree;
use crate::planner::tree::query_tree_node::QueryTreeNode;
use crate::planner::walker::selection::{FieldSelection, SelectionItem, SelectionSet};
use crate::state::supergraph_state::SubgraphName;
use petgraph::graph::{DiGraph, EdgeIndex, NodeIndex, NodeIndices};
use petgraph::visit::Bfs;
use petgraph::visit::{EdgeRef, NodeRef};
use petgraph::Directed;
use petgraph::Direction;
use std::collections::{HashSet, VecDeque};
use std::fmt::{Debug, Display};

use super::selection::Selection;

pub struct FetchGraph {
    graph: DiGraph<FetchStepData, ()>,
}

impl FetchGraph {
    pub fn new() -> Self {
        Self {
            graph: DiGraph::new(),
        }
    }
}

impl FetchGraph {
    pub fn parents_of(&self, index: NodeIndex) -> petgraph::graph::Edges<'_, (), Directed> {
        self.graph.edges_directed(index, Direction::Incoming)
    }

    pub fn children_of(&self, index: NodeIndex) -> petgraph::graph::Edges<'_, (), Directed> {
        self.graph.edges_directed(index, Direction::Outgoing)
    }

    pub fn step_indices(&self) -> NodeIndices {
        self.graph.node_indices()
    }

    pub fn get_step_data(&self, index: NodeIndex) -> Option<&FetchStepData> {
        self.graph.node_weight(index)
    }

    pub fn get_step_data_mut(&mut self, index: NodeIndex) -> Option<&mut FetchStepData> {
        self.graph.node_weight_mut(index)
    }

    pub fn connect(&mut self, parent_index: NodeIndex, child_index: NodeIndex) -> EdgeIndex {
        self.graph.add_edge(parent_index, child_index, ())
    }

    pub fn disconnect(&mut self, edge_index: EdgeIndex) -> bool {
        self.graph.remove_edge(edge_index).map_or(false, |_| true)
    }

    pub fn remove_step(&mut self, index: NodeIndex) -> bool {
        self.graph.remove_node(index).map_or(false, |_| true)
    }

    pub fn add_step(&mut self, data: FetchStepData) -> NodeIndex {
        self.graph.add_node(data)
    }

    pub fn bfs<F>(&self, root_index: NodeIndex, mut visitor: F) -> Option<NodeIndex>
    where
        F: FnMut(&NodeIndex, &FetchStepData) -> bool,
    {
        if self.graph.node_weight(root_index).is_none() {
            return None;
        }

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

    pub fn optimize(&mut self, root_step_index: NodeIndex) {
        self.deduplicate_and_prune_fetch_steps();
        self.merge_children_with_parents(root_step_index);
        self.merge_siblings(root_step_index);
        self.turn_mutations_into_sequence(root_step_index);
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
    fn deduplicate_and_prune_fetch_steps(&mut self) {
        for step_index in self.step_indices() {
            let step = self.get_step_data(step_index).unwrap();

            if step.output.selection_set.items.len() > 0
                && self.parents_of(step_index).next().is_some()
            {
                continue;
            }

            if self.children_of(step_index).next().is_some() {
                continue;
            }

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
                ) {
                    edges_to_remove.push(parent_to_step_edge.id());
                }
            }
        }

        for edge_index in edges_to_remove {
            self.disconnect(edge_index);
        }
    }

    fn merge_siblings(&mut self, root_step_index: NodeIndex) {
        let mut merges_to_perform: Vec<(NodeIndex, NodeIndex)> = Vec::new();

        self.bfs(root_step_index, |step_index, step_data| {
            let mut found = false;
            for parent_edge in self.parents_of(*step_index) {
                if found {
                    break;
                }

                let parent_index = parent_edge.source();
                for sibling_edge in self.children_of(parent_index) {
                    let sibling_index = sibling_edge.source();
                    let sibling = self.get_step_data(sibling_index).unwrap();
                    if sibling.can_merge(sibling_index, *step_index, step_data, self) {
                        merges_to_perform.push((sibling_index, *step_index));
                        found = true;
                        break;
                    }
                }
            }

            found
        });

        for (ancestor_index, step_index) in merges_to_perform {
            perform_fetch_step_merge(ancestor_index, step_index, self);
        }
    }

    fn merge_children_with_parents(&mut self, root_step_index: NodeIndex) {
        // Let's look from root to bottom
        // but also at parents at each level and try to merge those
        let mut merges_to_perform: Vec<(NodeIndex, NodeIndex)> = Vec::new();

        self.bfs(root_step_index, |step_index, step_data| {
            for ancestor_edge in self.parents_of(*step_index) {
                let ancestor_index = ancestor_edge.source();
                let ancestor = self.get_step_data(ancestor_index).unwrap();
                if ancestor.can_merge(ancestor_index, *step_index, step_data, self) {
                    merges_to_perform.push((ancestor_index, *step_index));
                }
            }

            false // returning false means we never stop the BFS before it finishes
        });

        for (ancestor_index, step_index) in merges_to_perform {
            perform_fetch_step_merge(ancestor_index, step_index, self);
        }
    }

    fn turn_mutations_into_sequence(&mut self, root_fetch_step_index: NodeIndex) {
        if !is_mutation_fetch_step(self, root_fetch_step_index) {
            return ();
        }

        let mut new_edges_pairs: Vec<(NodeIndex, NodeIndex)> = Vec::new();
        let mut edge_ids_to_remove: Vec<EdgeIndex> = Vec::new();
        let mut iter = self.children_of(root_fetch_step_index);
        let mut current = iter.next();

        while let Some(next_edge) = iter.next() {
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
            self.disconnect(edge_id);
        }
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
            let fetch_step = self.graph.node_weight(node_index).unwrap();

            fetch_step.pretty_write(f, node_index)?;
            write!(f, "\n")?;
        }

        writeln!(f, "\nTree:")?;

        let mut stack: Vec<(NodeIndex, usize)> = Vec::new();

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
                stack.push((child_index, 0));
            }
        }

        let mut visited: HashSet<NodeIndex> = HashSet::new();

        while let Some((node_index, depth)) = stack.pop() {
            if !visited.insert(node_index) {
                continue;
            }

            let indent = "  ".repeat(depth);
            writeln!(f, "{}[{}]", indent, node_index.index())?;

            for child_index in self
                .graph
                .edges_directed(node_index, Direction::Outgoing)
                .map(|edge_ref| edge_ref.target())
            {
                stack.push((child_index, depth + 1));
            }
        }

        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct FetchStepData {
    pub service_name: SubgraphName,
    pub response_path: Vec<String>,
    pub input: Selection,
    pub output: Selection,
    pub reserved_for_requires: Option<Selection>,
}

impl FetchStepData {
    pub fn is_reserved_for_requires(&self, selection: &Selection) -> bool {
        return self
            .reserved_for_requires
            .as_ref()
            .is_some_and(|requires| requires.eq(&selection));
    }

    pub fn pretty_write(
        &self,
        writer: &mut std::fmt::Formatter<'_>,
        index: NodeIndex,
    ) -> Result<(), std::fmt::Error> {
        write!(
            writer,
            "[{}] {}/{} {} → {} at $.{}",
            index.index(),
            self.input.type_name,
            self.service_name,
            self.input,
            self.output,
            self.response_path.join("."),
        )?;
        Ok(())
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

        // we should prevent merging steps with more than 1 parent
        // we should merge only if the parent of both is identical

        let mut self_path = self.response_path.clone();
        self_path.insert(0, "*".to_string());
        let mut other_path = other.response_path.clone();
        other_path.insert(0, "*".to_string());
        if self_path
            .iter()
            .enumerate()
            .all(|(i, p)| p.eq(&other_path[i]))
        {
            return false;
        }

        // if the `other` FetchStep has a single parent and it's `this` FetchStep
        if fetch_graph.parents_of(self_index).count() == 1
            && fetch_graph
                .parents_of(other_index)
                .any(|edge| edge.source() == self_index)
        {
            return false;
        }

        // if they do not share parents, they can't be merged
        if fetch_graph.parents_of(self_index).all(|self_edge| {
            fetch_graph
                .parents_of(other_index)
                .any(|other_edge| other_edge.source() == self_edge.source())
        }) {
            return false;
        }

        return true;
    }
}

fn perform_fetch_step_merge(
    self_index: NodeIndex,
    other_index: NodeIndex,
    fetch_graph: &mut FetchGraph,
) {
    // TODO: is there a way to not `clone` here?
    //       If I don't clone, the `me` variable yells at me about mutable vs immutable borrow
    let other = fetch_graph.get_step_data(other_index).unwrap().clone();
    let me = fetch_graph.get_step_data_mut(self_index).unwrap();
    me.output.add_at_path(
        &other.output,
        other.response_path[me.response_path.len()..].to_vec(),
        false,
    );

    let mut children_indexes: Vec<NodeIndex> = vec![];
    let mut parents_indexes: Vec<NodeIndex> = vec![];
    for edge_ref in fetch_graph.children_of(other_index) {
        children_indexes.push(edge_ref.target().id());
    }

    for edge_ref in fetch_graph.parents_of(other_index) {
        parents_indexes.push(edge_ref.source().id());
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
}

pub struct FetchSteps {
    pub root_step_index: Option<NodeIndex>,
}

fn create_noop_fetch_step(fetch_graph: &mut FetchGraph) -> NodeIndex {
    fetch_graph.add_step(FetchStepData {
        service_name: SubgraphName("*".to_string()),
        response_path: vec![],
        input: Selection {
            selection_set: SelectionSet { items: vec![] },
            type_name: "*".to_string(),
        },
        output: Selection {
            selection_set: SelectionSet { items: vec![] },
            type_name: "*".to_string(),
        },
        reserved_for_requires: None,
    })
}

fn create_fetch_step_for_entity_move(
    fetch_graph: &mut FetchGraph,
    subgraph_name: SubgraphName,
    type_name: String,
    response_path: &Vec<String>,
) -> NodeIndex {
    fetch_graph.add_step(FetchStepData {
        service_name: subgraph_name,
        response_path: response_path.clone(),
        input: Selection {
            selection_set: SelectionSet {
                items: vec![SelectionItem::Field(FieldSelection {
                    name: "__typename".to_string(),
                    alias: None,
                    is_leaf: true,
                    selections: SelectionSet { items: vec![] },
                })],
            },
            type_name: type_name.clone(),
        },
        output: Selection {
            selection_set: SelectionSet { items: vec![] },
            type_name: type_name,
        },
        reserved_for_requires: None,
    })
}

fn create_fetch_step_for_root_move(
    fetch_graph: &mut FetchGraph,
    subgraph_name: SubgraphName,
    type_name: String,
) -> NodeIndex {
    fetch_graph.add_step(FetchStepData {
        service_name: subgraph_name,
        response_path: vec![],
        input: Selection {
            selection_set: SelectionSet { items: vec![] },
            type_name: type_name.clone(),
        },
        output: Selection {
            selection_set: SelectionSet { items: vec![] },
            type_name: type_name,
        },
        reserved_for_requires: None,
    })
}

fn get_or_create_fetch_step_for_entity_move(
    fetch_graph: &mut FetchGraph,
    parent_fetch_step_index: NodeIndex,
    subgraph_name: SubgraphName,
    type_name: String,
    response_path: Vec<String>,
    key: Option<Selection>,
    requires: Option<Selection>,
) -> NodeIndex {
    let matching_child_index =
        fetch_graph
            .children_of(parent_fetch_step_index)
            .find_map(|to_child_edge_ref| {
                let fetch_step = fetch_graph
                    .get_step_data(to_child_edge_ref.target())
                    .unwrap();

                if fetch_step.service_name != subgraph_name {
                    return None;
                }

                if fetch_step.input.type_name != type_name {
                    return None;
                }

                if fetch_step.response_path != response_path {
                    return None;
                }

                if key.is_some() && fetch_step.input.contains(&key.as_ref().unwrap()) == false {
                    // requested key fields are not part of the input
                    return None;
                }

                // The `requirement` should be equal.
                // If it's a subset,
                //  then some other field in step's requirement may postpone/affect the child FetchStep.
                // If the FetchStep has no requirement,
                //  then attaching required fields to it may postpone/affect the resolution of other fields.
                if requires.is_some()
                    && fetch_step.is_reserved_for_requires(&requires.as_ref().unwrap()) == false
                {
                    return None;
                }

                return Some(to_child_edge_ref.target());
            });

    return match matching_child_index {
        Some(idx) => idx,
        None => {
            let step_index = create_fetch_step_for_entity_move(
                fetch_graph,
                subgraph_name,
                type_name.clone(),
                &response_path,
            );
            if let Some(selection) = key {
                let step = fetch_graph.get_step_data_mut(step_index).unwrap();
                step.input.add(selection)
            }

            return step_index;
        }
    };
}

fn process_children_for_fetch_steps(
    graph: &Graph,
    fetch_graph: &mut FetchGraph,
    query_node: &QueryTreeNode,
    parent_fetch_step_index: Option<NodeIndex>,
    response_path: &Vec<String>,
    fetch_path: &Vec<String>,
    requiring_fetch_step_index: Option<NodeIndex>,
) -> () {
    if query_node.children.is_empty() {
        return;
    }

    query_node.children.iter().for_each(|sub_step| {
        process_query_node(
            graph,
            fetch_graph,
            sub_step,
            parent_fetch_step_index,
            response_path,
            fetch_path,
            requiring_fetch_step_index,
        );
    });
}

fn process_requirements_for_fetch_steps(
    graph: &Graph,
    fetch_graph: &mut FetchGraph,
    query_node: &QueryTreeNode,
    parent_fetch_step_index: Option<NodeIndex>,
    requiring_fetch_step_index: Option<NodeIndex>,
    response_path: &Vec<String>,
    fetch_path: &Vec<String>,
) -> () {
    if query_node.requirements.is_empty() {
        return ();
    }

    query_node.requirements.iter().for_each(|req_query_node| {
        process_query_node(
            graph,
            fetch_graph,
            req_query_node,
            parent_fetch_step_index,
            response_path,
            fetch_path,
            requiring_fetch_step_index,
        );
        fetch_graph.connect(
            parent_fetch_step_index.unwrap(),
            requiring_fetch_step_index.unwrap(),
        );
    });
}

fn process_noop_edge(
    graph: &Graph,
    fetch_graph: &mut FetchGraph,
    query_node: &QueryTreeNode,
    parent_fetch_step_index: Option<NodeIndex>,
    response_path: &Vec<String>,
    fetch_path: &Vec<String>,
    requiring_fetch_step_index: Option<NodeIndex>,
) -> () {
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
    );
}

fn add_typename_field_to_output(
    fetch_step: &mut FetchStepData,
    type_name: String,
    add_at: Vec<String>,
) {
    fetch_step.output.add_at_path(
        &Selection {
            selection_set: SelectionSet {
                items: vec![SelectionItem::Field(FieldSelection {
                    name: "__typename".to_string(),
                    alias: None,
                    is_leaf: true,
                    selections: SelectionSet { items: vec![] },
                })],
            },
            type_name,
        },
        add_at,
        true,
    );
}

fn process_entity_move_edge(
    graph: &Graph,
    fetch_graph: &mut FetchGraph,
    query_node: &QueryTreeNode,
    parent_fetch_step_index: Option<NodeIndex>,
    response_path: &Vec<String>,
    fetch_path: &Vec<String>,
    requiring_fetch_step_index: Option<NodeIndex>,
    edge_index: EdgeIndex,
) -> () {
    if parent_fetch_step_index.is_none() {
        panic!("Expected a parent fetch step");
    }
    let edge = graph.edge(edge_index).unwrap();
    let requirement = match edge {
        Edge::EntityMove(em) => Selection {
            // TODO: actual selection set
            selection_set: SelectionSet { items: vec![] },
            type_name: em.requirements.type_name.clone(),
        },
        _ => panic!("Expected an entity move"),
    };

    let tail_node_index = graph.get_edge_tail(&edge_index).unwrap();
    let tail_node = graph.node(tail_node_index).unwrap();
    let type_name = match tail_node {
        Node::QueryRoot(t) => t,
        Node::MutationRoot(t) => t,
        Node::SubscriptionRoot(t) => t,
        Node::SubgraphType(t) => &t.name,
    };
    let graph_id = tail_node.graph_id().unwrap();

    let fetch_step_index = get_or_create_fetch_step_for_entity_move(
        fetch_graph,
        parent_fetch_step_index.unwrap(),
        SubgraphName(graph_id.to_string()),
        type_name.clone(),
        response_path.clone(),
        Some(requirement.clone()),
        None,
    );

    let fetch_step = fetch_graph.get_step_data_mut(fetch_step_index).unwrap();
    fetch_step.input.add(requirement);
    add_typename_field_to_output(fetch_step, type_name.clone(), fetch_path.clone());

    // Make the fetch step a child of the parent fetch step
    fetch_graph.connect(parent_fetch_step_index.unwrap(), fetch_step_index);

    process_requirements_for_fetch_steps(
        graph,
        fetch_graph,
        query_node,
        parent_fetch_step_index,
        Some(fetch_step_index),
        response_path,
        fetch_path,
    );
    process_children_for_fetch_steps(
        graph,
        fetch_graph,
        query_node,
        Some(fetch_step_index),
        response_path,
        &vec![],
        requiring_fetch_step_index,
    );
}

fn process_subgraph_entrypoint_edge(
    graph: &Graph,
    fetch_graph: &mut FetchGraph,
    query_node: &QueryTreeNode,
    parent_fetch_step_index: Option<NodeIndex>,
    subgraph_name: SubgraphName,
    type_name: String,
) {
    if parent_fetch_step_index.is_none() {
        panic!("Expected a parent fetch step")
    }

    let fetch_step_index = create_fetch_step_for_root_move(fetch_graph, subgraph_name, type_name);

    process_children_for_fetch_steps(
        graph,
        fetch_graph,
        query_node,
        Some(fetch_step_index),
        &vec![],
        &vec![],
        None,
    );

    fetch_graph.connect(parent_fetch_step_index.unwrap(), fetch_step_index);
}

fn process_plain_field_edge(
    graph: &Graph,
    fetch_graph: &mut FetchGraph,
    query_node: &QueryTreeNode,
    parent_fetch_step_index: Option<NodeIndex>,
    requiring_fetch_step_index: Option<NodeIndex>,
    response_path: &Vec<String>,
    fetch_path: &Vec<String>,
    field_name: String,
    field_is_leaf: bool,
    field_is_list: bool,
    field_type_name: String,
) {
    let parent_fetch_step_index = parent_fetch_step_index.expect("Expected a parent fetch step");

    if let Some(requiring_fetch_step_index) = requiring_fetch_step_index {
        fetch_graph.connect(parent_fetch_step_index, requiring_fetch_step_index);
    }

    let parent_fetch_step = fetch_graph
        .get_step_data_mut(parent_fetch_step_index)
        .unwrap();
    parent_fetch_step.output.add_at_path(
        &Selection {
            selection_set: SelectionSet {
                items: vec![SelectionItem::Field(FieldSelection {
                    name: field_name.clone(),
                    alias: None,
                    is_leaf: field_is_leaf,
                    selections: SelectionSet { items: vec![] },
                })],
            },
            type_name: field_type_name,
        },
        fetch_path.clone(),
        false,
    );

    let mut child_response_path = response_path.clone();
    let mut child_fetch_path = fetch_path.clone();
    child_response_path.push(field_name.clone());
    child_fetch_path.push(field_name.clone());

    if field_is_list {
        child_response_path.push("@".to_string());
        child_fetch_path.push("@".to_string());
    }

    process_children_for_fetch_steps(
        graph,
        fetch_graph,
        query_node,
        Some(parent_fetch_step_index),
        &child_response_path,
        &child_fetch_path,
        requiring_fetch_step_index,
    );
}

fn process_query_node(
    graph: &Graph,
    fetch_graph: &mut FetchGraph,
    query_node: &QueryTreeNode,
    parent_fetch_step_index: Option<NodeIndex>,
    response_path: &Vec<String>,
    fetch_path: &Vec<String>,
    requiring_fetch_step_index: Option<NodeIndex>,
) -> () {
    if query_node.edge_from_parent.is_none() {
        // return ();
        return process_noop_edge(
            graph,
            fetch_graph,
            query_node,
            parent_fetch_step_index,
            response_path,
            fetch_path,
            requiring_fetch_step_index,
        );
    }

    let edge_index = query_node.edge_from_parent.unwrap();
    let edge = graph.edge(edge_index).unwrap();

    match edge {
        Edge::SubgraphEntrypoint { name, .. } => {
            let tail_node_index = graph.get_edge_tail(&edge_index).unwrap();
            let tail_node = graph.node(tail_node_index).unwrap();
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
                name.clone(),
                type_name.clone(),
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
        Edge::FieldMove(field) => {
            if field.requirements.is_none() {
                process_plain_field_edge(
                    graph,
                    fetch_graph,
                    query_node,
                    parent_fetch_step_index,
                    requiring_fetch_step_index,
                    response_path,
                    fetch_path,
                    field.name.clone(),
                    field.is_leaf.clone(),
                    field.is_list.clone(),
                    field.type_name.clone(),
                )
            } else {
                todo!("not yet supported")
            }
        }
        Edge::AbstractMove(_) => {
            panic!("AbstractMove is not supported yet")
        }
    };
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

pub fn build_fetch_graph_from_query_tree(graph: &Graph, query_tree: QueryTree) -> FetchGraph {
    let mut fetch_graph = FetchGraph::new();

    process_query_node(
        graph,
        &mut fetch_graph,
        &query_tree.root,
        None,
        &vec![],
        &vec![],
        None,
    );

    let root_indexes = find_graph_roots(&fetch_graph);

    if root_indexes.is_empty() {
        panic!("No root FetchStep")
    }

    if root_indexes.len() > 1 {
        panic!("Expected a single root FetchStep")
    }

    fetch_graph.optimize(*root_indexes.first().unwrap());

    fetch_graph
}

fn is_mutation_fetch_step(fetch_graph: &mut FetchGraph, fetch_step_index: NodeIndex) -> bool {
    fetch_graph.children_of(fetch_step_index).all(|edge_ref| {
        let child = fetch_graph.get_step_data(edge_ref.target().id()).unwrap();
        return child.output.type_name.eq("Mutation");
    })
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
) -> bool {
    if !graph.get_step_data(child_index).is_some()
        || !graph.get_step_data(target_ancestor_index).is_some()
    {
        return false;
    }

    // Use a VecDeque for efficient queue operations (push_back, pop_front)
    let mut queue: VecDeque<NodeIndex> = VecDeque::new();
    // Use a HashSet to keep track of visited nodes during this specific search
    let mut visited: HashSet<NodeIndex> = HashSet::new();

    // Start BFS queue with all parents of `child_index` except `target_ancestor_index`
    for edge_ref in graph.parents_of(child_index) {
        let parent_index = edge_ref.source(); // Get the source node of the incoming edge

        if parent_index != target_ancestor_index {
            queue.push_back(parent_index);
            visited.insert(parent_index);
        }
    }

    // If no "other" parents were found to start the search from,
    // there's no indirect path, so return false immediately.
    if queue.is_empty() {
        return false;
    }

    // Perform BFS upwards (following incoming edges)
    while let Some(current_index) = queue.pop_front() {
        // If we reached the target ancestor indirectly, return true
        if current_index == target_ancestor_index {
            return true;
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
    false
}
