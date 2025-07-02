use crate::ast::merge_path::{Condition, MergePath, Segment};
use crate::ast::selection_item::SelectionItem;
use crate::ast::selection_set::{FieldSelection, InlineFragmentSelection, SelectionSet};
use crate::ast::type_aware_selection::TypeAwareSelection;
use crate::graph::edge::{Edge, FieldMove, InterfaceObjectTypeMove};
use crate::graph::node::Node;
use crate::graph::Graph;
use crate::planner::fetch::fetch_step_data::{FetchStepData, FetchStepKind};
use crate::planner::plan_nodes::{FetchNodePathSegment, FetchRewrite, ValueSetter};
use crate::planner::tree::query_tree::QueryTree;
use crate::planner::tree::query_tree_node::{MutationFieldPosition, QueryTreeNode};
use crate::planner::walker::path::OperationPath;
use crate::planner::walker::pathfinder::can_satisfy_edge;
use crate::state::supergraph_state::{SubgraphName, SupergraphState};
use petgraph::graph::EdgeReference;
use petgraph::stable_graph::{EdgeIndex, NodeIndex, NodeIndices, NodeReferences, StableDiGraph};
use petgraph::visit::EdgeRef;
use petgraph::visit::{Bfs, IntoNodeReferences};
use petgraph::Directed;
use petgraph::Direction;
use std::collections::VecDeque;
use std::fmt::{Debug, Display};
use tracing::{instrument, trace};

use super::error::FetchGraphError;

#[derive(Debug, Clone)]
pub struct FetchGraph {
    pub(crate) graph: StableDiGraph<FetchStepData, ()>,
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
        condition: None,
        kind: FetchStepKind::Root,
        input_rewrites: None,
        output_rewrites: None,
        variable_usages: None,
        variable_definitions: None,
        mutation_field_position: None,
        internal_aliases_locations: Vec::new(),
    })
}

fn create_fetch_step_for_entity_call(
    fetch_graph: &mut FetchGraph,
    subgraph_name: &SubgraphName,
    input_type_name: &str,
    output_type_name: &str,
    response_path: &MergePath,
    condition: Option<&Condition>,
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
        condition: condition.cloned(),
        kind: FetchStepKind::Entity,
        input_rewrites: None,
        output_rewrites: None,
        variable_usages: None,
        variable_definitions: None,
        mutation_field_position: None,
        internal_aliases_locations: Vec::new(),
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
        condition: None,
        kind: FetchStepKind::Root,
        variable_usages: None,
        variable_definitions: None,
        input_rewrites: None,
        output_rewrites: None,
        mutation_field_position,
        internal_aliases_locations: Vec::new(),
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
    condition: Option<&Condition>,
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

                    if fetch_step.condition.as_ref() != condition {
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
                condition,
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
    condition: Option<&Condition>,
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

                    if fetch_step.condition.as_ref() != condition {
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
                condition,
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
    condition: Option<&Condition>,
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
            condition,
            created_from_requires,
        )?);
    }

    Ok(leaf_fetch_step_indexes)
}

// TODO: simplfy args
#[allow(clippy::too_many_arguments)]
#[instrument(level = "trace",skip_all, fields(
  count = query_node.requirements.len()
))]
fn process_requirements_for_fetch_steps(
    graph: &Graph,
    fetch_graph: &mut FetchGraph,
    query_node: &QueryTreeNode,
    parent_fetch_step_index: NodeIndex,
    requiring_fetch_step_index: Option<NodeIndex>,
    condition: Option<&Condition>,
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
            condition,
            true,
        )?;
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
    condition: Option<&Condition>,
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
        condition,
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
    condition: Option<&Condition>,
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
        condition,
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
                FetchNodePathSegment::TypenameEquals(output_type_name.to_string()),
                FetchNodePathSegment::Key("__typename".to_string()),
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
        condition,
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
        condition,
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
    condition: Option<&Condition>,
    created_from_requires: bool,
) -> Result<Vec<NodeIndex>, FetchGraphError> {
    if fetch_graph.parents_of(parent_fetch_step_index).count() != 1 {
        return Err(FetchGraphError::NonSingleParent(
            parent_fetch_step_index.index(),
        ));
    }
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

    let head_node_index = graph.get_edge_head(&edge_index)?;
    let head_node = graph.node(head_node_index)?;
    let head_subgraph_name = match head_node {
        Node::SubgraphType(t) => &t.subgraph,
        _ => return Err(FetchGraphError::ExpectedSubgraphType),
    };

    let tail_node_index = graph.get_edge_tail(&edge_index)?;
    let tail_node = graph.node(tail_node_index)?;
    let (interface_type_name, subgraph_name) = match tail_node {
        Node::SubgraphType(t) => (&t.name, &t.subgraph),
        // todo: FetchGraphError::MissingSubgraphName(tail_node.clone())
        _ => return Err(FetchGraphError::ExpectedSubgraphType),
    };

    let step_for_children_index = ensure_fetch_step_for_subgraph(
        fetch_graph,
        parent_fetch_step_index,
        subgraph_name,
        object_type_name,
        interface_type_name,
        response_path,
        Some(&requirement),
        None,
        condition,
        created_from_requires,
    )?;

    let step_for_children = fetch_graph.get_step_data_mut(step_for_children_index)?;
    trace!(
        "adding input requirement '{}' to fetch step [{}]",
        requirement,
        step_for_children_index.index()
    );
    step_for_children.input.add(&requirement);
    let key_to_reenter_subgraph =
        find_satisfiable_key(graph, query_node.requirements.first().unwrap())?;
    step_for_children.input.add(&requirement);
    trace!(
        "adding key '{}' to fetch step [{}]",
        key_to_reenter_subgraph,
        step_for_children_index.index()
    );
    step_for_children.input.add(key_to_reenter_subgraph);

    trace!(
        "adding input rewrite '... on {} {{ __typename }}' to '{}'",
        interface_type_name,
        interface_type_name
    );
    step_for_children.add_input_rewrite(FetchRewrite::ValueSetter(ValueSetter {
        path: vec![
            FetchNodePathSegment::TypenameEquals(interface_type_name.to_string()),
            FetchNodePathSegment::Key("__typename".to_string()),
        ],
        set_value_to: interface_type_name.clone().into(),
    }));

    let parent_fetch_step = fetch_graph.get_step_data_mut(parent_fetch_step_index)?;
    add_typename_field_to_output(parent_fetch_step, interface_type_name, fetch_path);

    // In all cases it's `__typename` that needs to be resolved by another subgraph.
    trace!("Creating a fetch step for requirement of @interfaceObject");
    let step_for_requirements_index = create_fetch_step_for_entity_call(
        fetch_graph,
        head_subgraph_name,
        object_type_name,
        interface_type_name,
        response_path,
        condition,
        false,
    );
    let step_for_requirements = fetch_graph.get_step_data_mut(step_for_requirements_index)?;
    trace!(
        "Adding {} to fetch([{}]).input",
        key_to_reenter_subgraph,
        step_for_requirements_index.index()
    );
    step_for_requirements.input.add(key_to_reenter_subgraph);

    //
    // Given `f0 { ... on User { f1 } }` where f1 is a field contributed by @interfaceObject,
    // the requirement to resolve `f1` with `User` type condition is a real value of `__typename` field.
    // - step_for_requirements    -> __typename
    // - step_for_children        -> f1
    // - parent                   -> f0
    //
    // parent -> step_for_requirements -> step_for_children
    //
    // f0 -> __typename -> f1
    //

    fetch_graph.connect(parent_fetch_step_index, step_for_requirements_index);

    trace!("Processing requirements");
    let leaf_fetch_step_indexes = process_query_node(
        graph,
        fetch_graph,
        query_node.requirements.first().unwrap(),
        Some(step_for_requirements_index),
        response_path,
        &MergePath::default(),
        None,
        condition,
        true,
    )?;

    if leaf_fetch_step_indexes.is_empty() {
        fetch_graph.connect(step_for_requirements_index, step_for_children_index);
    } else {
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
        response_path,
        &MergePath::default(),
        requiring_fetch_step_index,
        condition,
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
    condition: Option<&Condition>,
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
                        skip_if: None,
                        include_if: None,
                    }),
                ],
            },
            type_name: head_type_name.clone(),
        },
        fetch_path.clone(),
        false,
    );

    let child_response_path = response_path.push(Segment::Cast(target_type_name.clone(), None));
    let child_fetch_path = fetch_path.push(Segment::Cast(target_type_name.clone(), None));

    process_children_for_fetch_steps(
        graph,
        fetch_graph,
        query_node,
        parent_fetch_step_index,
        &child_response_path,
        &child_fetch_path,
        requiring_fetch_step_index,
        condition,
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
    condition: Option<&Condition>,
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
    let mut child_response_path = response_path.push(Segment::Field(
        child_segment.to_string(),
        segment_args_hash,
        None,
    ));
    let mut child_fetch_path = fetch_path.push(Segment::Field(
        child_segment.to_string(),
        segment_args_hash,
        None,
    ));

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
        condition,
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
    condition: Option<&Condition>,
    created_from_requires: bool,
) -> Result<Vec<NodeIndex>, FetchGraphError> {
    if fetch_graph.parents_of(parent_fetch_step_index).count() != 1 {
        return Err(FetchGraphError::NonSingleParent(
            parent_fetch_step_index.index(),
        ));
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

    let condition = query_node.condition.as_ref().or(condition);
    trace!("Condition: {:?}", condition);

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
        condition,
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
        "Adding {} to fetch([{}]).input (requires)",
        requires,
        step_for_children_index.index()
    );
    step_for_children.input.add(requires);
    trace!(
        "Adding {} to fetch([{}]).input (key re-enter)",
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
        condition,
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

    let mut child_response_path =
        response_path.push(Segment::Field(field_move.name.clone(), 0, None));
    let mut child_fetch_path =
        MergePath::default().push(Segment::Field(field_move.name.clone(), 0, None));

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
        condition,
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
        condition,
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
    condition: Option<&Condition>,
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
                condition,
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
                    condition,
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
                    condition,
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
                condition,
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
                condition,
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
            condition,
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

#[instrument(level = "trace", skip(graph, query_tree, supergraph), fields(
    requirements_count = query_tree.root.requirements.len(),
    children_count = query_tree.root.children.len(),
))]
pub fn build_fetch_graph_from_query_tree(
    graph: &Graph,
    supergraph: &SupergraphState,
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
    fetch_graph.optimize(supergraph)?;
    fetch_graph.collect_variable_usages()?;

    trace!("fetch graph after optimizations:");
    trace!("{}", fetch_graph);

    Ok(fetch_graph)
}
