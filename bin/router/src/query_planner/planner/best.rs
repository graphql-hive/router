//! This module solves the problem of finding the "best" execution plan for a
//! query that can be satisfied in many different ways by various subgraphs.
//!
//! The "best" plan is defined by a cost model that penalizes crossing
//! subgraph boundaries, and favors plans that are "local" to a single subgraph.
//!
//! The query is broken down into decision points, one per leaf. Each has a set of
//! `Alternatives`, and each alternative is a `Candidate`: one path that resolves the leaf.
//! The plan picks one `Candidate` per set.
//!
//! The picking is greedy. Sets go from fewest choices to most, and each one takes the candidate
//! that adds the least cost to the plan built so far. Candidates are sorted by their own cost
//! first, so a tie goes to the cheaper one on its own.
//!
//! This used to be followed by a branch-and-bound search, but it never ran. Its bound was the sum
//! of each set's cheapest candidate on its own, and merging a candidate into the plan always costs
//! less than that, at least the shared root. So the greedy cost was always under the bound and
//! the search stopped at the root, for every query. A real search needs a valid bound and a budget.
//!
//! Candidates aren't turned into trees to be costed. A candidate's tree is a chain, a node per
//! path segment, so both its own cost and what it adds to the plan come straight from the path.
//! Only the picked candidate of each set becomes a tree, to merge into the plan.

use std::rc::Rc;

use petgraph::graph::NodeIndex;

use crate::query_planner::{
    graph::{edge::Edge, Graph},
    planner::{
        error::QueryPlanError,
        tree::{
            query_tree::QueryTree,
            query_tree_node::{merge_query_tree_node_list, MutationFieldPosition, QueryTreeNode},
        },
        walker::{
            path::{same_selection_attributes, OperationPath, PathSegment},
            ResolvedOperation,
        },
    },
    state::supergraph_state::OperationKind,
    utils::cancellation::CancellationToken,
};

/// The high-penalty cost for crossing a subgraph boundary or satisfying a requirement.
/// This is the primary driver of the optimization, encouraging plans with fewer subgraphs.
const CROSS_SUBGRAPH_COST: u64 = 1000;
const FIELD_COST: u64 = 1;

/// One possible way to resolve a leaf of the query. There's one per path the walker found, so
/// it keeps only what rebuilds the path's segments, not the whole path.
struct Candidate {
    root_node: NodeIndex,
    last_segment: Option<Rc<PathSegment>>,
    mutation_field_position: MutationFieldPosition,
    /// The cost of the candidate's tree on its own.
    cost: u64,
}

/// Represents a set of alternative Candidates to satisfy a single leaf in the query.
/// The final plan must choose exactly one fragment from each ChoiceGroup.
type Alternatives = Vec<Candidate>;

impl Candidate {
    fn new(
        graph: &Graph<'_>,
        path: OperationPath<'_>,
        mutation_field_position: MutationFieldPosition,
    ) -> Result<Self, QueryPlanError> {
        let segments = path.get_segments();
        // The tree's root, then the chain.
        let cost = FIELD_COST + chain_cost(graph, &segments, 0, &context_trees(&segments))?;
        let candidate = Self {
            root_node: path.root_node,
            last_segment: path.last_segment,
            mutation_field_position,
            cost,
        };

        #[cfg(test)]
        assert_eq!(
            candidate.cost,
            calculate_cost_of_tree(graph, &candidate.build_tree(graph)?.root),
            "the cost from the path must match the cost of the tree"
        );

        Ok(candidate)
    }

    /// The path again, minus what the walker needed (union context, cost).
    fn path(&self) -> OperationPath<'static> {
        OperationPath {
            root_node: self.root_node,
            last_segment: self.last_segment.clone(),
            cost: 0,
            union_context: None,
        }
    }

    fn build_tree(&self, graph: &Graph<'_>) -> Result<QueryTree, QueryPlanError> {
        Ok(QueryTree::from_path(
            graph,
            &self.path(),
            self.mutation_field_position,
        )?)
    }
}

/// Each `Alternatives` group represents a set of possible ways to satisfy one leaf of the query.
fn prepare_alternatives(
    graph: &Graph<'_>,
    operation: ResolvedOperation<'_>,
) -> Result<Vec<Alternatives>, QueryPlanError> {
    let is_mutation = matches!(operation.operation_kind, OperationKind::Mutation);
    let mut per_leaf_alternatives_asc: Vec<Alternatives> = Vec::new();

    for (index, root_field_options) in operation.root_field_groups.into_iter().enumerate() {
        let mutation_field_position: MutationFieldPosition = is_mutation.then_some(index);

        for paths_to_leaf in root_field_options {
            // Sized up front, collecting through `Result` would grow it and leave spare capacity.
            let mut alternatives = Alternatives::with_capacity(paths_to_leaf.len());
            for path in paths_to_leaf {
                alternatives.push(Candidate::new(graph, path, mutation_field_position)?);
            }
            // Cheapest on its own first, it wins ties in `pick_candidates`.
            alternatives.sort_by_key(|candidate| candidate.cost);
            per_leaf_alternatives_asc.push(alternatives);
        }
    }

    // Sort alternatives by length in ascending order.
    per_leaf_alternatives_asc.sort_by_key(|alternatives| alternatives.len());

    Ok(per_leaf_alternatives_asc)
}

pub fn find_best_combination(
    graph: &Graph<'_>,
    operation: ResolvedOperation,
    cancellation_token: &CancellationToken,
) -> Result<QueryTree, QueryPlanError> {
    if operation.root_field_groups.is_empty()
        || operation
            .root_field_groups
            .iter()
            .any(|paths_to_leafs| paths_to_leafs.iter().any(Vec::is_empty))
    {
        return Err(QueryPlanError::EmptyPlan);
    }

    let per_leaf_alternatives_asc = prepare_alternatives(graph, operation)?;
    pick_candidates(graph, per_leaf_alternatives_asc, cancellation_token)?
        .ok_or(QueryPlanError::EmptyPlan)
}

/// For each set, the candidate that adds the least cost to the plan so far.
/// Takes the sets by value, so each one's paths are freed once it's picked.
fn pick_candidates(
    graph: &Graph<'_>,
    alternatives_list: Vec<Alternatives>,
    cancellation_token: &CancellationToken,
) -> Result<Option<QueryTree>, QueryPlanError> {
    let mut plan: Option<QueryTree> = None;

    for alternatives in alternatives_list {
        cancellation_token.bail_if_cancelled()?;

        let mut best_added_cost = u64::MAX;
        let mut best: Option<&Candidate> = None;
        for candidate in &alternatives {
            let added_cost = match plan.as_ref() {
                Some(plan) => added_cost_of_candidate(graph, &plan.root, candidate)?,
                None => candidate.cost,
            };
            if added_cost < best_added_cost {
                best_added_cost = added_cost;
                best = Some(candidate);
            }
        }

        let Some(best) = best else {
            return Ok(None);
        };
        let tree = best.build_tree(graph)?;
        match plan.as_mut() {
            Some(plan) => Rc::make_mut(&mut plan.root).merge_nodes(&tree.root),
            None => plan = Some(tree),
        }
    }

    Ok(plan)
}

/// What merging `candidate`'s tree into the tree at `root` would add, worked out from the path.
/// The same as `calculate_added_cost_of_merge`, without building the candidate's tree.
fn added_cost_of_candidate(
    graph: &Graph<'_>,
    root: &QueryTreeNode,
    candidate: &Candidate,
) -> Result<u64, QueryPlanError> {
    let path = candidate.path();
    let segments = path.get_segments();
    let context = context_trees(&segments);
    let mut added_cost = 0;
    let mut target = root;

    // Down the chain while the plan has the same node, those only add what their requirement
    // adds. The first node the plan doesn't have comes in whole, with the rest of the chain.
    for (index, segment) in segments.iter().enumerate() {
        let mutation_field_position = if index == 0 {
            candidate.mutation_field_position
        } else {
            None
        };
        let tail_node = graph.get_edge_tail(&segment.edge_index)?;
        let same_node = target.children.iter().find(|child| {
            child.node_index == tail_node
                && child.edge_from_parent == Some(segment.edge_index)
                && same_selection_attributes(
                    &child.selection_attributes,
                    &segment.selection_attributes,
                )
                && child.mutation_field_position == mutation_field_position
                && child.condition.as_ref() == segment.condition.as_deref()
                // nodes built from a path have no coordinate, only context trees do
                && child.context_coordinate().is_none()
        });

        match same_node {
            Some(same_node) => {
                if let Some(requirement) = &segment.requirement_tree {
                    added_cost += match same_node
                        .requirements
                        .iter()
                        .find(|existing| existing.as_ref() == requirement.as_ref())
                    {
                        Some(existing) => {
                            calculate_added_cost_of_merge(graph, existing, requirement)
                        }
                        None => CROSS_SUBGRAPH_COST + calculate_cost_of_tree(graph, requirement),
                    };
                }
                added_cost += context_cost(graph, Some(same_node), segment, &context, index);
                target = same_node;
            }
            None => {
                added_cost += chain_cost(graph, &segments, index, &context)?;
                break;
            }
        }
    }

    #[cfg(test)]
    assert_eq!(
        added_cost,
        calculate_added_cost_of_merge(graph, root, &candidate.build_tree(graph)?.root),
        "the added cost from the path must match the added cost of the tree"
    );

    Ok(added_cost)
}

/// The cost of the chain of tree nodes the segments from `start` become, entering the first one
/// included. A node costs a field, plus a subgraph crossing when its edge enters a subgraph, plus
/// its requirement and its `@fromContext` bits, like in `calculate_cost_of_tree`.
fn chain_cost(
    graph: &Graph<'_>,
    segments: &[&PathSegment],
    start: usize,
    context: &[(usize, Rc<QueryTreeNode>)],
) -> Result<u64, QueryPlanError> {
    let mut cost = 0;
    for (index, segment) in segments.iter().enumerate().skip(start) {
        // Building the tree looks the tail up too, and fails the same way.
        graph.get_edge_tail(&segment.edge_index)?;

        cost += FIELD_COST;
        if matches!(
            graph.edge(segment.edge_index)?,
            Edge::SubgraphEntrypoint { .. }
        ) {
            cost += CROSS_SUBGRAPH_COST;
        }
        if let Some(requirement) = &segment.requirement_tree {
            cost += CROSS_SUBGRAPH_COST + calculate_cost_of_tree(graph, requirement);
        }
        cost += context_cost(graph, None, segment, context, index);
    }
    Ok(cost)
}

/// The context trees of the path, with the index of the segment whose node they hang on, in the
/// order `QueryTreeNode::from_path_segment_sequences` adds them. Empty, so free, for most paths.
fn context_trees(segments: &[&PathSegment]) -> Vec<(usize, Rc<QueryTreeNode>)> {
    let mut trees = Vec::new();
    for (index, segment) in segments.iter().enumerate() {
        for requirement in segment.context_requirements() {
            // a tree pointing at its own segment or before the first one doesn't make it into the
            // tree either
            if let Some(node) = index.checked_sub(requirement.segments_back) {
                if node < index {
                    trees.push((node, Rc::clone(&requirement.tree)));
                }
            }
        }
    }
    trees
}

/// What the `@fromContext` bits of the node of segment `index` cost: its context trees, merged
/// like in the tree, and one hop for resolving any context. When `target`, the same node in the
/// plan, is there, only what merging into it adds, like in `calculate_added_cost_of_merge`.
fn context_cost(
    graph: &Graph<'_>,
    target: Option<&QueryTreeNode>,
    segment: &PathSegment,
    context: &[(usize, Rc<QueryTreeNode>)],
    index: usize,
) -> u64 {
    let mut cost = 0;
    if !segment.context_requirements().is_empty()
        && target.is_none_or(|target| target.resolved_context_requirements().is_empty())
    {
        cost += CROSS_SUBGRAPH_COST;
    }

    if !context.iter().any(|(node, _)| *node == index) {
        return cost;
    }
    let mut trees = Vec::new();
    for (_, tree) in context.iter().filter(|(node, _)| *node == index) {
        merge_query_tree_node_list(&mut trees, std::slice::from_ref(tree));
    }
    let existing = target.map_or(&[][..], |target| target.context_requirements());
    for tree in &trees {
        cost += match existing
            .iter()
            .find(|existing| existing.as_ref() == tree.as_ref())
        {
            Some(existing) => calculate_added_cost_of_merge(graph, existing, tree),
            None => calculate_cost_of_tree(graph, tree),
        };
    }
    cost
}

/// Calculate how much cost `source` adds when merged into `target`.
///
/// Only nodes missing from `target`, and their missing descendants and requirements,
/// contribute additional cost. This avoids re-costing the entire merged tree while
/// retaining the same cost as a full merge followed by `calculate_cost_of_tree`.
fn calculate_added_cost_of_merge(
    graph: &Graph<'_>,
    target: &QueryTreeNode,
    source: &QueryTreeNode,
) -> u64 {
    // context trees merge like requirements, but `calculate_cost_of_tree` charges no hop for them,
    // just one hop for having any resolved context at all
    let context_cost: u64 = source
        .context_requirements()
        .iter()
        .map(|source_node| {
            match target
                .context_requirements()
                .iter()
                .find(|target_node| target_node.as_ref() == source_node.as_ref())
            {
                Some(target_node) => calculate_added_cost_of_merge(graph, target_node, source_node),
                None => calculate_cost_of_tree(graph, source_node),
            }
        })
        .sum();
    let resolved_context_cost = if target.resolved_context_requirements().is_empty()
        && !source.resolved_context_requirements().is_empty()
    {
        CROSS_SUBGRAPH_COST
    } else {
        0
    };

    calculate_added_cost_for_node_list(graph, &target.children, &source.children, false)
        + calculate_added_cost_for_node_list(
            graph,
            &target.requirements,
            &source.requirements,
            true,
        )
        + context_cost
        + resolved_context_cost
}

fn calculate_added_cost_for_node_list(
    graph: &Graph<'_>,
    target_list: &[Rc<QueryTreeNode>],
    source_list: &[Rc<QueryTreeNode>],
    is_requirement: bool,
) -> u64 {
    source_list
        .iter()
        .map(|source_node| {
            if let Some(target_node) = target_list
                .iter()
                .find(|target_node| target_node.as_ref() == source_node.as_ref())
            {
                calculate_added_cost_of_merge(graph, target_node, source_node)
            } else if is_requirement {
                CROSS_SUBGRAPH_COST + calculate_cost_of_tree(graph, source_node)
            } else {
                edge_cost(graph, source_node) + calculate_cost_of_tree(graph, source_node)
            }
        })
        .sum()
}

#[inline(always)]
fn calculate_cost_of_tree(graph: &Graph<'_>, node: &QueryTreeNode) -> u64 {
    let mut current_cost = FIELD_COST;

    for child in &node.children {
        if edge_cost(graph, child) > 0 {
            current_cost += CROSS_SUBGRAPH_COST;
        }

        current_cost += calculate_cost_of_tree(graph, child);
    }

    for requirement in &node.requirements {
        current_cost += CROSS_SUBGRAPH_COST;
        current_cost += calculate_cost_of_tree(graph, requirement);
    }

    for requirement in node.context_requirements() {
        current_cost += calculate_cost_of_tree(graph, requirement);
    }
    if !node.resolved_context_requirements().is_empty() {
        current_cost += CROSS_SUBGRAPH_COST;
    }

    current_cost
}

#[inline(always)]
fn edge_cost(graph: &Graph<'_>, node: &QueryTreeNode) -> u64 {
    if node.edge_from_parent.is_some_and(|edge_index| {
        matches!(
            graph.edge(edge_index).expect("edge should exist"),
            Edge::SubgraphEntrypoint { .. }
        )
    }) {
        CROSS_SUBGRAPH_COST
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use petgraph::graph::NodeIndex;

    use super::*;
    use crate::query_planner::planner::tree::query_tree_node::NodeContext;
    use crate::query_planner::planner::walker::path::ContextRequirement;

    fn node(
        index: usize,
        children: Vec<Rc<QueryTreeNode>>,
        requirements: Vec<Rc<QueryTreeNode>>,
    ) -> Rc<QueryTreeNode> {
        Rc::new(QueryTreeNode {
            node_index: NodeIndex::new(index),
            edge_from_parent: None,
            requirements,
            context: None,
            children,
            selection_attributes: None,
            condition: None,
            mutation_field_position: None,
        })
    }

    #[test]
    fn incremental_merge_cost_matches_full_tree_cost() {
        let graph = Graph::default();
        // The graph is only consulted for edge costs. These test nodes have no graph edges.

        let shared_leaf = node(2, vec![], vec![]);
        let shared_requirement = node(3, vec![], vec![]);
        let target_shared = node(1, vec![shared_leaf], vec![shared_requirement]);
        let target = QueryTreeNode {
            node_index: NodeIndex::new(0),
            edge_from_parent: None,
            requirements: vec![],
            context: None,
            children: vec![target_shared],
            selection_attributes: None,
            condition: None,
            mutation_field_position: None,
        };

        let source_shared = node(
            1,
            vec![node(2, vec![], vec![]), node(4, vec![], vec![])],
            vec![node(3, vec![], vec![]), node(5, vec![], vec![])],
        );
        let source = QueryTreeNode {
            node_index: NodeIndex::new(0),
            edge_from_parent: None,
            requirements: vec![],
            context: Some(Box::new(NodeContext {
                requirements: vec![node(7, vec![], vec![])],
                coordinate: None,
                resolved_requirements: vec![Rc::new(ContextRequirement {
                    argument: 0,
                    selection: 0,
                    levels: 1,
                    segments_back: 1,
                    cost: 0,
                    tree: node(8, vec![], vec![]),
                })],
            })),
            children: vec![source_shared, node(6, vec![], vec![])],
            selection_attributes: None,
            condition: None,
            mutation_field_position: None,
        };

        let current_cost = calculate_cost_of_tree(&graph, &target);
        let added_cost = calculate_added_cost_of_merge(&graph, &target, &source);
        let mut merged = target.clone();
        merged.merge_nodes(&source);

        assert_eq!(
            added_cost,
            calculate_cost_of_tree(&graph, &merged) - current_cost
        );
    }
}
