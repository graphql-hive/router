use std::rc::Rc;
use std::{cmp, fmt::Debug};

use petgraph::{
    graph::{EdgeIndex, NodeIndex},
    visit::EdgeRef,
};

use crate::query_planner::ast::merge_path::Condition;
use crate::query_planner::graph::node::UnionMembersData;
use crate::query_planner::planner::walker::pathfinder::NavigationTarget;
use crate::query_planner::{
    ast::{arguments::ArgumentsMap, selection_set::FieldSelection},
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

impl SelectionAttributes {
    /// The attributes of `field`, to share between the paths through it. Most fields have no
    /// alias and no arguments, those all get the same `Rc`, so nothing is allocated for them.
    pub fn shared(field: &FieldSelection) -> Rc<Self> {
        thread_local! {
            static NONE: Rc<SelectionAttributes> = Rc::new(SelectionAttributes::default());
        }

        if field.alias.is_none() && field.arguments.is_none() {
            return NONE.with(Rc::clone);
        }
        Rc::new(Self {
            alias: field.alias.clone(),
            arguments: field.arguments.clone(),
        })
    }
}

/// Paths through the same field share its attributes, so the same `Rc` means equal,
/// without comparing the arguments.
pub fn same_selection_attributes(
    left: &Option<Rc<SelectionAttributes>>,
    right: &Option<Rc<SelectionAttributes>>,
) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => Rc::ptr_eq(left, right) || left == right,
        (None, None) => true,
        _ => false,
    }
}

#[derive(Debug, Clone)]
pub struct ContextRequirement {
    pub argument: usize,
    pub selection: usize,
    pub levels: usize,
    pub segments_back: usize,
    pub cost: u64,
    pub tree: Rc<QueryTreeNode>,
}

#[derive(Debug, Clone)]
pub struct PathSegment {
    // Link to the previous step, null for the first segment originating from rootNode
    prev: Option<Rc<PathSegment>>,
    /// How many segments the path has up to and including this one.
    /// Lets the getters below allocate once, with the right size.
    depth: usize,
    pub edge_index: EdgeIndex,
    tail_node: NodeIndex,
    cumulative_cost: u64,
    pub requirement_tree: Option<Rc<QueryTreeNode>>,
    // `None` when empty, and cloning it is a refcount bump
    context_requirements: Option<Rc<[Rc<ContextRequirement>]>>,
    /// Shared by every path through the same field, see `NavigationTarget::Field`.
    pub selection_attributes: Option<Rc<SelectionAttributes>>,
    /// Boxed, it's rare (`@skip`/`@include`) and every segment would pay for two `String`s.
    pub condition: Option<Box<Condition>>,
}

impl PathSegment {
    pub fn context_requirements(&self) -> &[Rc<ContextRequirement>] {
        self.context_requirements.as_deref().unwrap_or_default()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PossibleUnionMembers<'graph> {
    All(&'graph [&'graph str]),
    One(&'graph str),
}

impl<'graph> PossibleUnionMembers<'graph> {
    fn contains(&self, member_type_name: &str) -> bool {
        match self {
            Self::All(members) => members.contains(&member_type_name),
            Self::One(member) => *member == member_type_name,
        }
    }

    fn len(&self) -> usize {
        match self {
            Self::All(members) => members.len(),
            Self::One(_) => 1,
        }
    }

    fn as_vec(&self) -> Vec<&'graph str> {
        match self {
            Self::All(members) => members.to_vec(),
            Self::One(member) => vec![member],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnionContext<'graph> {
    data: &'graph UnionMembersData<'graph>,
    pub graph_id: &'graph str,
    pub member_name: &'graph str,
    possible_members: PossibleUnionMembers<'graph>,
}

impl<'graph> UnionContext<'graph> {
    pub(crate) fn can_resolve_member(&self, member_type_name: &str) -> bool {
        self.possible_members.contains(member_type_name)
    }

    pub fn possible_members_len(&self) -> usize {
        self.possible_members.len()
    }

    pub fn eq_field(&self, other: &Self) -> bool {
        self.data.type_name == other.data.type_name && self.data.field_name == other.data.field_name
    }

    pub fn possible_members(&self) -> Vec<&'graph str> {
        self.possible_members.as_vec()
    }

    pub fn set_possible_member(&mut self, possible_member: &'graph str) {
        self.possible_members = PossibleUnionMembers::One(possible_member);
    }

    fn narrow_to_member(&self, member_type_name: &'graph str) -> Self {
        Self {
            data: self.data,
            graph_id: self.graph_id,
            member_name: member_type_name,
            possible_members: PossibleUnionMembers::One(member_type_name),
        }
    }
}

impl PathSegment {
    pub fn new_root(edge: &EdgeReference) -> Self {
        Self {
            prev: None,
            depth: 1,
            edge_index: edge.id(),
            tail_node: edge.target(),
            cumulative_cost: edge.weight().cost(),
            requirement_tree: None,
            context_requirements: None,
            selection_attributes: None,
            condition: None,
        }
    }
}

#[derive(Clone)]
pub struct OperationPath<'graph> {
    pub root_node: NodeIndex,
    pub last_segment: Option<Rc<PathSegment>>,
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
    pub fn new(root_node_index: NodeIndex, last_segment: Option<Rc<PathSegment>>) -> Self {
        Self {
            root_node: root_node_index,
            cost: last_segment
                .as_ref()
                .map_or(0, |segment| segment.cumulative_cost),
            last_segment,
            union_context: None,
        }
    }

    pub fn new_entrypoint(edge: &EdgeReference<'_>) -> Self {
        // The first "segment" conceptually starts after the first edge from root
        let path_segment = PathSegment::new_root(edge);
        let arc_path_segment = Rc::new(path_segment);

        OperationPath::new(edge.source(), Some(arc_path_segment))
    }

    pub fn advance(
        &self,
        graph: &'graph Graph<'graph>,
        edge_ref: &EdgeReference<'graph>,
        requirement: Option<Rc<QueryTreeNode>>,
        context_requirements: Vec<Rc<ContextRequirement>>,
        target: &NavigationTarget,
    ) -> OperationPath<'graph> {
        let prev_cost = self.cost;
        let edge_cost = edge_ref.weight().cost();
        let new_cost = prev_cost
            + edge_cost
            + context_requirements
                .iter()
                .map(|requirement| requirement.cost)
                .sum::<u64>();

        let new_segment_data = PathSegment {
            prev: self.last_segment.clone(),
            depth: self.last_segment.as_ref().map_or(0, |prev| prev.depth) + 1,
            tail_node: edge_ref.target(),
            edge_index: edge_ref.id(),
            cumulative_cost: new_cost,
            requirement_tree: requirement,
            context_requirements: (!context_requirements.is_empty())
                .then(|| context_requirements.into()),
            selection_attributes: match target {
                NavigationTarget::Field { attributes, .. } => Some(Rc::clone(attributes)),
                NavigationTarget::ConcreteType(_, _) => None,
            },
            condition: match target {
                NavigationTarget::Field { field, .. } => {
                    Option::<Condition>::from(*field).map(Box::new)
                }
                NavigationTarget::ConcreteType(_, condition) => condition.clone().map(Box::new),
            },
        };
        let new_segment = Rc::new(new_segment_data);

        let union_context = match edge_ref.weight() {
            Edge::FieldMove(field_move) if field_move.is_leaf => None,
            Edge::FieldMove(_) => {
                let tail = graph.node(edge_ref.target()).ok();
                let union_data = tail.and_then(|tail| tail.union_members_data());
                let graph_id = tail.and_then(|tail| tail.graph_id());

                match (union_data, graph_id) {
                    (Some(union_data), Some(graph_id)) => Some(UnionContext {
                        data: union_data,
                        graph_id,
                        member_name: union_data.object_type_name,
                        possible_members: PossibleUnionMembers::All(
                            union_data.possible_members.as_slice(),
                        ),
                    }),
                    _ => None,
                }
            }
            Edge::AbstractMove(member_type_name) => self
                .union_context
                .as_ref()
                .map(|scope| scope.narrow_to_member(member_type_name)),
            Edge::EntityMove(_) | Edge::InterfaceObjectTypeMove(_) | Edge::ReentryMove(_) => None,
            _ => self.union_context.clone(),
        };

        OperationPath {
            root_node: self.root_node,
            cost: new_cost,
            last_segment: Some(new_segment),
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

    pub fn before_segment(&self, graph: &'graph Graph, segment: &PathSegment) -> Self {
        let mut prefix = Self::new(self.root_node, segment.prev.clone());
        let tail = graph.node(prefix.tail()).expect("path node exists");
        if let (Some(data), Some(graph_id)) = (tail.union_members_data(), tail.graph_id()) {
            prefix.union_context = Some(UnionContext {
                data,
                graph_id,
                member_name: data.object_type_name,
                possible_members: PossibleUnionMembers::All(&data.possible_members),
            });
        }
        prefix
    }

    /// The segments from the last one back to the first, without collecting them.
    pub fn segments_rev(&self) -> impl Iterator<Item = &PathSegment> {
        std::iter::successors(self.last_segment.as_deref(), |segment| {
            segment.prev.as_deref()
        })
    }

    /// The segments from the root to the tail.
    pub fn get_segments(&self) -> Vec<&PathSegment> {
        self.collect_from_root(|segment| segment)
    }

    pub fn get_edges(&self) -> Vec<EdgeIndex> {
        self.collect_from_root(|segment| segment.edge_index)
    }

    pub fn get_requirement_tree(&self) -> Vec<Option<Rc<QueryTreeNode>>> {
        self.collect_from_root(|segment| segment.requirement_tree.clone())
    }

    /// Walks the segments from the tail by reference, into one exactly-sized Vec, and flips it.
    fn collect_from_root<'a, T>(&'a self, map: impl Fn(&'a PathSegment) -> T) -> Vec<T> {
        let mut items = Vec::with_capacity(self.last_segment.as_ref().map_or(0, |s| s.depth));
        let mut current = self.last_segment.as_deref();

        while let Some(segment) = current {
            items.push(map(segment));
            current = segment.prev.as_deref();
        }

        items.reverse();
        items
    }

    pub fn pretty_print(&self, graph: &Graph<'_>) -> String {
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
        let source_segments = self.get_segments();
        let target_segments = other.get_segments();

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
                depth: previous_new_segment.as_ref().map_or(0, |prev| prev.depth) + 1,
                prev: previous_new_segment.take(),
                cumulative_cost: new_cumulative_cost,
                edge_index: original_segment.edge_index,
                requirement_tree: original_segment.requirement_tree.clone(),
                context_requirements: original_segment.context_requirements.clone(),
                tail_node: original_segment.tail_node,
                selection_attributes: original_segment.selection_attributes.clone(),
                condition: original_segment.condition.clone(),
            };

            previous_new_segment = Some(Rc::new(new_segment_data));
        }

        OperationPath::new(new_root_node.unwrap(), previous_new_segment)
            // The requirement continuation reuses other's tail, so it also must reuse its union context
            .with_union_context(other.union_context.clone())
    }

    fn with_union_context(mut self, scope: Option<UnionContext<'graph>>) -> Self {
        self.union_context = scope;
        self
    }
}
