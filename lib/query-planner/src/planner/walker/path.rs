use std::collections::VecDeque;
use std::rc::Rc;
use std::{cmp, collections::HashSet, fmt::Debug};

use petgraph::{
    graph::{EdgeIndex, NodeIndex},
    visit::EdgeRef,
};

use crate::ast::merge_path::Condition;
use crate::planner::walker::pathfinder::NavigationTarget;
use crate::{
    ast::arguments::ArgumentsMap,
    graph::{edge::Edge, edge::EdgeReference, Graph},
    planner::tree::query_tree_node::QueryTreeNode,
};

/// This structure contains attributes from the original selection set that was part of the incoming operation.
#[derive(Debug, Clone, Default)]
pub struct SelectionAttributes {
    pub alias: Option<String>,
    pub arguments: Option<ArgumentsMap>,
}

impl PartialEq for SelectionAttributes {
    fn eq(&self, other: &Self) -> bool {
        self.alias == other.alias && self.arguments == other.arguments
    }
}

#[derive(Debug, Clone)]
pub struct PathSegment {
    // Link to the previous step, null for the first segment originating from rootNode
    prev: Option<Rc<PathSegment>>,
    pub edge_index: EdgeIndex,
    tail_node: NodeIndex,
    cumulative_cost: u64,
    pub requirement_tree: Option<Rc<QueryTreeNode>>,
    pub selection_attributes: Option<SelectionAttributes>,
    pub condition: Option<Condition>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnionContext<'graph> {
    pub parent_type_name: &'graph str,
    pub field_name: &'graph str,
    pub graph_id: &'graph str,
    pub member_name: &'graph str,
    pub possible_members: Vec<&'graph str>,
}

impl<'graph> UnionContext<'graph> {
    fn can_resolve_member(&self, member_type_name: &str) -> bool {
        self.possible_members.contains(&member_type_name)
    }

    pub fn eq_field(&self, other: &Self) -> bool {
        self.parent_type_name == other.parent_type_name && self.field_name == other.field_name
    }

    fn narrow_to_member(&self, member_type_name: &'graph str) -> Self {
        Self {
            parent_type_name: self.parent_type_name,
            field_name: self.field_name,
            graph_id: self.graph_id,
            member_name: member_type_name,
            possible_members: vec![member_type_name],
        }
    }
}

impl PathSegment {
    pub fn new_root(edge: &EdgeReference) -> Self {
        Self {
            prev: None,
            edge_index: edge.id(),
            tail_node: edge.target(),
            cumulative_cost: edge.weight().cost(),
            requirement_tree: None,
            selection_attributes: None,
            condition: None,
        }
    }
}

#[derive(Clone)]
pub struct OperationPath<'graph> {
    pub root_node: NodeIndex,
    pub last_segment: Option<Rc<PathSegment>>,
    pub visited_edge_indices: Rc<HashSet<EdgeIndex>>,
    pub cost: u64,
    // The union context for the current path, if any.
    // If we hit a field returning a union, this will be set to the union context.
    // If we hit a field returning a non-union type, this will be cleared.
    pub union_context: Option<UnionContext<'graph>>,
}

impl Debug for OperationPath<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut out = f.debug_struct("");
        let mut out = out.field("cost", &self.cost);
        let edges = self.get_edges();

        if edges.is_empty() {
            out = out.field("empty", &true).field("head", &self.root_node);
        } else {
            out = out.field(
                "egdes",
                &edges
                    .iter()
                    .map(|i| format!("{:?}", i))
                    .collect::<Vec<String>>()
                    .join(" --> "),
            );
        }
        out.finish()
    }
}

impl<'graph> OperationPath<'graph> {
    pub fn new(
        root_node_index: NodeIndex,
        last_segment: Option<Rc<PathSegment>>,
        visited_edge_indices: Rc<HashSet<EdgeIndex>>,
    ) -> Self {
        Self {
            root_node: root_node_index,
            cost: last_segment
                .as_ref()
                .map_or(0, |segment| segment.cumulative_cost),
            last_segment,
            visited_edge_indices,
            union_context: None,
        }
    }

    pub fn new_entrypoint(edge: &EdgeReference<'_>) -> Self {
        // The first "segment" conceptually starts after the first edge from root
        let path_segment = PathSegment::new_root(edge);
        let arc_path_segment = Rc::new(path_segment);
        let visited_set: Rc<HashSet<EdgeIndex>> = Rc::new([edge.id()].into_iter().collect());

        OperationPath::new(edge.source(), Some(arc_path_segment), visited_set)
    }

    pub fn advance(
        &self,
        graph: &'graph Graph,
        edge_ref: &EdgeReference<'graph>,
        requirement: Option<Rc<QueryTreeNode>>,
        target: &NavigationTarget,
    ) -> OperationPath<'graph> {
        let prev_cost = self.cost;
        let edge_cost = edge_ref.weight().cost();
        let new_cost = prev_cost + edge_cost;
        let mut new_visited = self.visited_edge_indices.clone();
        Rc::make_mut(&mut new_visited).insert(edge_ref.id());

        let new_segment_data = PathSegment {
            prev: self.last_segment.clone(),
            tail_node: edge_ref.target(),
            edge_index: edge_ref.id(),
            cumulative_cost: new_cost,
            requirement_tree: requirement,
            selection_attributes: match target {
                NavigationTarget::Field { field, .. } => Some(SelectionAttributes {
                    alias: field.alias.clone(),
                    arguments: field.arguments.clone(),
                }),
                NavigationTarget::ConcreteType(_, _) => None,
            },
            condition: match target {
                NavigationTarget::Field { field, .. } => (*field).into(),
                NavigationTarget::ConcreteType(_, condition) => condition.clone(),
            },
        };
        let new_segment = Rc::new(new_segment_data);

        let union_context = match edge_ref.weight() {
            Edge::FieldMove(_) => {
                let tail = graph.node(edge_ref.target()).ok();
                let union_data = tail.and_then(|tail| tail.union_members_data());
                let graph_id = tail.and_then(|tail| tail.graph_id());

                match (union_data, graph_id) {
                    (Some(union_data), Some(graph_id)) => Some(UnionContext {
                        parent_type_name: &union_data.type_name,
                        field_name: &union_data.field_name,
                        graph_id,
                        member_name: &union_data.object_type_name,
                        possible_members: union_data
                            .possible_members
                            .iter()
                            .map(|member| member.as_str())
                            .collect(),
                    }),
                    _ => None,
                }
            }
            Edge::AbstractMove(member_type_name) => self
                .union_context
                .as_ref()
                .map(|scope| scope.narrow_to_member(member_type_name)),
            Edge::EntityMove(_) | Edge::InterfaceObjectTypeMove(_) => None,
            _ => self.union_context.clone(),
        };

        OperationPath {
            root_node: self.root_node,
            cost: new_cost,
            last_segment: Some(new_segment),
            visited_edge_indices: new_visited,
            union_context,
        }
    }

    pub fn can_resolve_union_member(&self, member_type_name: &str) -> bool {
        self.union_context
            .as_ref()
            // If the union context is not present, it means we're not resolving things
            // for the field returning a union type,
            // therefore we don't limit members.
            .is_none_or(|scope| scope.can_resolve_member(member_type_name))
    }

    pub fn tail(&self) -> NodeIndex {
        self.last_segment
            .as_ref()
            .map_or(self.root_node, |segment| segment.tail_node)
    }

    pub fn has_visited_edge(&self, edge_index: &EdgeIndex) -> bool {
        self.visited_edge_indices.contains(edge_index)
    }

    pub fn get_segments(&self) -> Vec<Rc<PathSegment>> {
        let mut segments: VecDeque<Rc<PathSegment>> = VecDeque::new();
        let mut current: Option<Rc<PathSegment>> = self.last_segment.clone();

        while let Some(segment) = current {
            segments.push_front(segment.clone());
            current = segment.prev.clone();
        }

        segments.into_iter().collect()
    }

    pub fn get_edges(&self) -> Vec<EdgeIndex> {
        let mut edges: VecDeque<EdgeIndex> = VecDeque::new();
        let mut current: Option<Rc<PathSegment>> = self.last_segment.clone();

        while let Some(segment) = current {
            edges.push_front(segment.edge_index);
            current = segment.prev.clone();
        }

        edges.into_iter().collect()
    }

    pub fn get_requirement_tree(&self) -> Vec<Option<Rc<QueryTreeNode>>> {
        let mut requirement_tree_vec: VecDeque<Option<Rc<QueryTreeNode>>> = VecDeque::new();
        let mut current: Option<Rc<PathSegment>> = self.last_segment.clone();

        while let Some(segment) = current {
            requirement_tree_vec.push_front(segment.requirement_tree.clone());
            current = segment.prev.clone();
        }

        requirement_tree_vec.into_iter().collect()
    }

    pub fn pretty_print(&self, graph: &Graph) -> String {
        let edges = self.get_edges();

        if edges.is_empty() {
            graph.node(self.root_node).unwrap().display_name()
        } else {
            edges
                .iter()
                .enumerate()
                .map(|(vec_index, edge_index)| graph.pretty_print_edge(*edge_index, vec_index > 0))
                .collect::<Vec<String>>()
                .join(" ")
        }
    }

    /**
     * Given an original path (source) and a path found to satisfy a requirement (target),
     * this function identifies the point where `target` diverges from `source` and
     * returns a new OperationPath representing only the divergent suffix of `target`.
     * The new path's root node will be the tail node of the last common segment,
     * and its cost will be relative to that divergence point.
     *
     * self: The original path from which the requirement check started.
     * other: The path found that satisfies (part of) the requirement.
     */
    pub fn build_requirement_continuation_path(&self, other: &OperationPath<'graph>) -> Self {
        let source_segments: Vec<Rc<PathSegment>> = self.get_segments();
        let target_segments: Vec<Rc<PathSegment>> = other.get_segments();

        // Index of the last common segment in the sequence
        let mut common_index: Option<usize> = None;
        let len = cmp::min(source_segments.len(), target_segments.len());

        for index in 0..len {
            if source_segments[index].edge_index == target_segments[index].edge_index {
                common_index = Some(index);
            } else {
                // Stop at the first difference
                break;
            }
        }

        let new_root_node: Option<NodeIndex>;
        let cost_offset: Option<u64>;

        match common_index {
            // No common segments after the initial root node.
            None => {
                new_root_node = Some(self.root_node);
                cost_offset = Some(0);
            }
            // The new path starts after the last common segment.
            // Its root is the tail node of that common segment.
            Some(common_idx) => {
                let last_common_segment = &target_segments[common_idx];
                new_root_node = Some(last_common_segment.tail_node);
                cost_offset = Some(last_common_segment.cumulative_cost);
            }
        }

        // Rebuild the suffix segments list from the target path
        let mut previous_new_segment: Option<Rc<PathSegment>> = None;

        for original_segment in target_segments
            .iter()
            .skip(common_index.map(|v| v + 1).unwrap_or(0))
        {
            // Cost relative to the new root node
            let new_cumulative_cost = original_segment.cumulative_cost - cost_offset.unwrap_or(0);

            let new_segment_data = PathSegment {
                prev: previous_new_segment.take(),
                cumulative_cost: new_cumulative_cost,
                edge_index: original_segment.edge_index,
                requirement_tree: original_segment.requirement_tree.clone(),
                tail_node: original_segment.tail_node,
                selection_attributes: original_segment.selection_attributes.clone(),
                condition: original_segment.condition.clone(),
            };

            previous_new_segment = Some(Rc::new(new_segment_data));
        }

        OperationPath::new(
            new_root_node.unwrap(),
            previous_new_segment,
            self.visited_edge_indices.clone(),
        )
        // The requirement continuation reuses other's tail, so it also must reuse its union context
        .with_union_context(other.union_context.clone())
    }

    fn with_union_context(mut self, scope: Option<UnionContext<'graph>>) -> Self {
        self.union_context = scope;
        self
    }
}
