use std::{cmp, collections::HashSet, fmt::Debug, mem};

use petgraph::{
    graph::{EdgeIndex, NodeIndex},
    visit::EdgeRef,
};

use crate::{
    graph::{edge::EdgeReference, Graph},
    planner::tree::query_tree_node::QueryTreeNode,
};

#[derive(Debug, Clone)]
pub struct PathSegment {
    // Link to the previous step, null for the first segment originating from rootNode
    prev: Option<Box<PathSegment>>,
    edge_index: EdgeIndex,
    tail_node: NodeIndex,
    cumulative_cost: u64,
    requirement_tree: Option<QueryTreeNode>,
}

impl PathSegment {
    pub fn new_root(edge: &EdgeReference) -> Self {
        Self {
            prev: None,
            edge_index: edge.id(),
            tail_node: edge.target(),
            cumulative_cost: edge.weight().cost(),
            requirement_tree: None,
        }
    }
}

#[derive(Clone)]
pub struct OperationPath {
    pub root_node: NodeIndex,
    pub last_segment: Option<PathSegment>,
    pub visited_edge_indices: HashSet<EdgeIndex>,
    pub cost: u64,
}

impl Debug for OperationPath {
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

impl OperationPath {
    pub fn new(
        root_node_index: NodeIndex,
        last_segment: Option<PathSegment>,
        visited_edge_indices: HashSet<EdgeIndex>,
    ) -> Self {
        Self {
            root_node: root_node_index,
            cost: last_segment
                .as_ref()
                .map_or(0, |segment| segment.cumulative_cost),
            last_segment,
            visited_edge_indices,
        }
    }

    pub fn new_entrypoint(edge: &EdgeReference<'_>) -> Self {
        // The first "segment" conceptually starts after the first edge from root
        let path_segment = PathSegment::new_root(edge);
        let visited_set: HashSet<EdgeIndex> = [edge.id()].into_iter().collect();

        OperationPath::new(edge.source(), Some(path_segment), visited_set)
    }

    pub fn advance(
        &self,
        edge_ref: &EdgeReference<'_>,
        requirement: Option<QueryTreeNode>,
    ) -> OperationPath {
        let prev_cost = self.cost;
        let edge_cost = edge_ref.weight().cost();
        let new_cost = prev_cost + edge_cost;
        let mut new_visited = self.visited_edge_indices.clone();
        new_visited.insert(edge_ref.id());

        let new_segment = PathSegment {
            prev: self.last_segment.as_ref().map(|r| Box::new(r.clone())),
            tail_node: edge_ref.target(),
            edge_index: edge_ref.id(),
            cumulative_cost: new_cost,
            requirement_tree: requirement,
        };

        OperationPath::new(self.root_node, Some(new_segment), new_visited)
    }

    pub fn tail(&self) -> NodeIndex {
        self.last_segment
            .as_ref()
            .map_or(self.root_node, |segment| segment.tail_node)
    }

    pub fn has_visited_edge(&self, edge_index: &EdgeIndex) -> bool {
        self.visited_edge_indices.contains(edge_index)
    }

    pub fn get_edges(&self) -> Vec<EdgeIndex> {
        // TODO: Consider VecDeque so we can just do push_front
        let mut edges: Vec<EdgeIndex> = vec![];

        let mut current = self.last_segment.as_ref();
        while let Some(segment) = current {
            edges.push(segment.edge_index);
            current = segment.prev.as_deref();
        }

        edges.reverse();
        edges
    }

    pub fn get_requirement_tree(&self) -> Vec<Option<&QueryTreeNode>> {
        // TODO: Consider VecDeque so we can just do push_front
        let mut requirement_tree: Vec<Option<&QueryTreeNode>> = vec![];

        let mut current = self.last_segment.as_ref();
        while let Some(segment) = current {
            requirement_tree.push(segment.requirement_tree.as_ref());
            current = segment.prev.as_deref();
        }

        requirement_tree.reverse();
        requirement_tree
    }

    pub fn get_segments(&self) -> Vec<&PathSegment> {
        // TODO: Consider VecDeque so we can just do push_front
        let mut segments: Vec<&PathSegment> = vec![];

        let mut current = self.last_segment.as_ref();
        while let Some(segment) = current {
            segments.push(segment);
            current = segment.prev.as_deref();
        }

        segments.reverse();
        segments
    }

    pub fn pretty_print(&self, graph: &Graph) -> String {
        let edges = self.get_edges();

        if edges.is_empty() {
            graph.node(self.root_node).unwrap().id()
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
    pub fn build_requirement_continuation_path(&self, other: &Self) -> Self {
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
            Some(common_index) => {
                let last_common_segment = target_segments[common_index];
                new_root_node = Some(last_common_segment.tail_node);
                cost_offset = Some(last_common_segment.cumulative_cost);
            }
        }

        // Rebuild the suffix segments list from the target path
        let mut previous_new_segment: Option<Box<PathSegment>> = None;

        for original_segment in target_segments
            .iter()
            .skip(common_index.map(|v| v + 1).unwrap_or(0))
        {
            // Cost relative to the new root node
            let new_cumulative_cost = original_segment.cumulative_cost - cost_offset.unwrap_or(0);

            let new_segment = PathSegment {
                // TODO: Apologize
                prev: mem::take(&mut previous_new_segment),
                cumulative_cost: new_cumulative_cost,
                edge_index: original_segment.edge_index,
                requirement_tree: original_segment.requirement_tree.clone(),
                tail_node: original_segment.tail_node,
            };

            previous_new_segment = Some(Box::new(new_segment));
        }

        OperationPath::new(
            new_root_node.unwrap(),
            previous_new_segment.map(|b| *b), // unbox
            self.visited_edge_indices.clone(),
        )
    }
}
