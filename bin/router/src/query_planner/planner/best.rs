//! This module solves the problem of finding the "best" execution plan for a
//! query that can be satisfied in many different ways by various subgraphs.
//!
//! The "best" plan is defined by a cost model that penalizes crossing
//! subgraph boundaries, and favors plans that are "local" to a single subgraph.
//!
//! A naive search to find the best combination would be exponentially slow (we were there...),
//! as the number of possible plans is the product of the number of choices for each part of
//! the query. This module implements a search algorithm to make this problem solved.
//!
//! The core of the solution is a Branch and Bound algorithm implemented over a
//! Depth-First Search (DFS). This approach explores all possible
//! combinations while aggressively pruning entire branches of the search space that
//! are guaranteed to not contain the optimal solution.
//!
//! The key components are:
//!
//! 1.  The query is broken down into independent decision points.
//!     For each point, there is a set of `Alternatives`, and each alternative is a
//!     `Candidate` plan fragment. The goal is to pick exactly one `Candidate` from
//!     each set of `Alternatives`. Leaves with a single way to get them are in every plan,
//!     so they're merged up front into a `BasePlan`.
//!
//! 2.  Before the main search, a fast greedy search (`Search::greedy`)
//!     is run to find a "good enough" initial plan.
//!     This provides a strong `best_cost` value, which is critical for making the
//!     pruning in the main search much more effective.
//!
//! 3.  The Bounding Function (`min_remaining_costs`) - this is the most important
//!     optimization. We pre-compute a vector that contains an optimistic,
//!     "best-case" cost for completing the plan from any given step to the end.
//!     It never overestimates the true cost, which allows
//!     for the pruning check: if a partial plan's cost plus the optimistic
//!     remaining cost is already worse than our best-found solution, we can safely
//!     abandon the entire branch.
//!
//! 4.  To find good solutions faster, we sort both the
//!     decision points and the choices themselves:
//!     -   Sets of `Alternatives` are sorted by size (fewest choices first) to reduce
//!         the branching factor and find a complete plan sooner.
//!     -   `Candidates` within each set are sorted by their individual cost to explore
//!         the most promising options first.
//!
//! 5.  The `Candidate` struct uses `OnceCell` to ensure that the
//!     potentially expensive work of building a `QueryTree` is only ever
//!     done if a candidate is actually evaluated, and never more than once.
//!
//! 6.  Candidate combinations are costed incrementally (`Costs`). Since adding a candidate only
//!     adds nodes not already present in the current tree, we avoid recalculating the
//!     cost of the entire merged tree while preserving the exact branch-and-bound costs.

use petgraph::graph::NodeIndex;
use std::{cell::OnceCell, rc::Rc};

use crate::query_planner::{
    graph::{edge::Edge, error::GraphError, Graph},
    planner::{
        error::QueryPlanError,
        tree::{
            query_tree::QueryTree,
            query_tree_node::{merge_query_tree_node_list, MutationFieldPosition, QueryTreeNode},
        },
        walker::{
            path::{OperationPath, PathSegment},
            ResolvedOperation,
        },
    },
    state::supergraph_state::OperationKind,
    utils::cancellation::CancellationToken,
};

type PathAndPosition<'graph> = (OperationPath<'graph>, MutationFieldPosition);

/// The high-penalty cost for crossing a subgraph boundary or satisfying a requirement.
/// This is the primary driver of the optimization, encouraging plans with fewer subgraphs.
const CROSS_SUBGRAPH_COST: u64 = 1000;
const FIELD_COST: u64 = 1;

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

    let leaves = Leaves::of(operation);
    if leaves.singles.is_empty() && leaves.groups.is_empty() {
        return Err(QueryPlanError::EmptyPlan);
    }

    let base = BasePlan::merge(graph, leaves.singles, cancellation_token)?;
    let groups = leaves
        .groups
        .into_iter()
        .map(|paths| Candidate::alternatives(graph, paths))
        .collect::<Result<Vec<_>, _>>()?;

    let best_case_cost_per_leaf: Vec<u64> = base
        .leaf_costs
        .iter()
        .copied()
        .chain(
            groups
                .iter()
                .map(|alternatives| alternatives.first().map_or(u64::MAX, |c| c.cost)),
        )
        .collect();
    let min_remaining_costs = calculate_min_remaining_costs(&best_case_cost_per_leaf);

    let mut search = Search {
        graph,
        groups: &groups,
        min_remaining_costs: &min_remaining_costs[base.leaf_costs.len()..],
        cancellation_token,
        best_cost: u64::MAX,
        best_tree: None,
    };

    // A strong initial `best_cost` is critical for effective pruning.
    search.greedy(base.tree.clone(), base.cost());

    // The same bounds the search would check on its way through the single leaves.
    for (index, pair) in base.costs_before.windows(2).enumerate() {
        if pair[0] + min_remaining_costs[index] >= search.best_cost || pair[1] >= search.best_cost {
            return search.best_tree.ok_or(QueryPlanError::EmptyPlan);
        }
    }

    let base_cost = base.cost();
    search.explore(0, base.tree, base_cost)?;

    search.best_tree.ok_or(QueryPlanError::EmptyPlan)
}

/// The ways to get each leaf of the query.
struct Leaves<'graph> {
    /// Leaves with one way to get them, in the order they came in.
    singles: Vec<PathAndPosition<'graph>>,
    /// The ways to get each of the other leaves, the leaves with fewest ways first.
    groups: Vec<Vec<PathAndPosition<'graph>>>,
}

impl<'graph> Leaves<'graph> {
    fn of(operation: ResolvedOperation<'graph>) -> Self {
        let is_mutation = matches!(operation.operation_kind, OperationKind::Mutation);
        let mut singles = Vec::new();
        let mut groups = Vec::new();

        for (index, root_field_options) in operation.root_field_groups.into_iter().enumerate() {
            let mutation_field_position: MutationFieldPosition = is_mutation.then_some(index);

            for mut paths_to_leaf in root_field_options {
                if paths_to_leaf.len() == 1 {
                    let path = paths_to_leaf.pop().expect("one path");
                    singles.push((path, mutation_field_position));
                    continue;
                }
                groups.push(
                    paths_to_leaf
                        .into_iter()
                        .map(|path| (path, mutation_field_position))
                        .collect::<Vec<_>>(),
                );
            }
        }

        groups.sort_by_key(|alternatives| alternatives.len());

        Self { singles, groups }
    }
}

/// The leaves with one way to get them, merged into one tree, as they're in every plan.
///
/// Merged once, into a tree nobody else holds, so merging doesn't copy nodes. Searching
/// through them instead keeps a version of the tree per leaf, and every merge copies the
/// nodes it goes through.
struct BasePlan {
    tree: Option<QueryTree>,
    /// The cost before each leaf is merged, and after the last one.
    costs_before: Vec<u64>,
    /// What each leaf costs on its own.
    leaf_costs: Vec<u64>,
}

impl BasePlan {
    fn merge(
        graph: &Graph<'_>,
        singles: Vec<PathAndPosition<'_>>,
        cancellation_token: &CancellationToken,
    ) -> Result<Self, QueryPlanError> {
        let mut base = BasePlan {
            tree: None,
            costs_before: Vec::with_capacity(singles.len() + 1),
            leaf_costs: Vec::with_capacity(singles.len()),
        };
        base.costs_before.push(0);

        for (path, mutation_field_position) in singles {
            cancellation_token.bail_if_cancelled()?;
            let candidate = Candidate::new(graph, path, mutation_field_position)?;
            base.leaf_costs.push(candidate.cost);
            let cost = base.cost();
            match base.tree.as_mut() {
                Some(tree) => {
                    let root = Rc::make_mut(&mut tree.root);
                    base.costs_before
                        .push(cost + candidate.added_cost(graph, root)?);
                    candidate.merge_into(graph, root)?;
                }
                None => {
                    base.costs_before.push(cost + candidate.cost);
                    base.tree = Some(candidate.tree(graph)?);
                }
            }
        }

        Ok(base)
    }

    fn cost(&self) -> u64 {
        *self.costs_before.last().expect("starts with a zero")
    }
}

/// A potential piece of the final query plan.
/// It represents one of many possible ways to resolve a part of the query.
///
/// Costing and merging work on the path itself. Its `QueryTree` is built only if the search
/// gets to it, which it rarely does, and building one per candidate was most of the memory.
struct Candidate<'graph> {
    path: OperationPath<'graph>,
    mutation_field_position: MutationFieldPosition,
    /// Paths with `@fromContext` requirements always go through their `QueryTree`,
    /// the context trees merge into each other.
    uses_context: bool,
    /// What the tree of this path costs on its own.
    cost: u64,
    tree: OnceCell<QueryTree>,
}

/// Represents a set of alternative Candidates to satisfy a single leaf in the query.
/// The final plan must choose exactly one candidate from each set.
type Alternatives<'graph> = Vec<Candidate<'graph>>;

impl<'graph> Candidate<'graph> {
    fn new(
        graph: &Graph<'_>,
        path: OperationPath<'graph>,
        mutation_field_position: MutationFieldPosition,
    ) -> Result<Self, GraphError> {
        let uses_context = path
            .segments_rev()
            .any(|segment| !segment.context_requirements().is_empty());
        let tree = OnceCell::new();
        let cost = if uses_context {
            let built = QueryTree::from_path(graph, &path, mutation_field_position)?;
            let cost = Costs(graph).tree(&built.root);
            let _ = tree.set(built);
            cost
        } else {
            FIELD_COST + Costs(graph).of_segments(&path.get_segments())?
        };
        Ok(Self {
            path,
            mutation_field_position,
            uses_context,
            cost,
            tree,
        })
    }

    /// The candidates for one leaf, cheapest first, as trying cheaper options first is more
    /// likely to lead to better solutions sooner.
    fn alternatives(
        graph: &Graph<'_>,
        paths: Vec<PathAndPosition<'graph>>,
    ) -> Result<Alternatives<'graph>, GraphError> {
        let mut alternatives = paths
            .into_iter()
            .map(|(path, position)| Candidate::new(graph, path, position))
            .collect::<Result<Alternatives, _>>()?;
        alternatives.sort_by_key(|candidate| candidate.cost);
        Ok(alternatives)
    }

    fn tree(&self, graph: &Graph<'_>) -> Result<QueryTree, GraphError> {
        if let Some(tree) = self.tree.get() {
            return Ok(tree.clone());
        }
        let tree = QueryTree::from_path(graph, &self.path, self.mutation_field_position)?;
        Ok(self.tree.get_or_init(|| tree).clone())
    }

    /// How much merging this candidate into `root` adds to its cost.
    fn added_cost(&self, graph: &Graph<'_>, root: &QueryTreeNode) -> Result<u64, GraphError> {
        if self.uses_context {
            return Ok(Costs(graph).added_by_merge(root, &self.tree(graph)?.root));
        }
        Costs(graph).added_by_path(
            root,
            &self.path.get_segments(),
            self.mutation_field_position,
        )
    }

    fn merge_into(&self, graph: &Graph<'_>, root: &mut QueryTreeNode) -> Result<(), GraphError> {
        if self.uses_context {
            root.merge_nodes(&self.tree(graph)?.root);
            return Ok(());
        }
        merge_path(
            graph,
            root,
            &self.path.get_segments(),
            self.mutation_field_position,
        )
    }
}

/// Pre-compute a "suffix lower bound" (LB) to enable effective pruning.
/// `min_remaining_costs[i]` stores an optimistic "best-case" cost for completing the
/// plan from leaf `i` to the end, from the best-case cost of each leaf.
/// In `Search::explore`, we can then check:
/// `cost_so_far + min_remaining_costs[depth] >= best_cost_so_far`.
/// If this is true, we know the current path is a dead end and can be abandoned.
fn calculate_min_remaining_costs(best_case_cost_per_leaf: &[u64]) -> Vec<u64> {
    let mut min_remaining_costs = vec![0; best_case_cost_per_leaf.len() + 1];
    for i in (0..best_case_cost_per_leaf.len()).rev() {
        min_remaining_costs[i] = min_remaining_costs[i + 1] + best_case_cost_per_leaf[i];
    }
    min_remaining_costs
}

/// Picks one candidate per group, keeping the cheapest complete plan it finds.
struct Search<'a, 'graph> {
    graph: &'a Graph<'a>,
    groups: &'a [Alternatives<'graph>],
    /// The best-case cost from each group to the end.
    min_remaining_costs: &'a [u64],
    cancellation_token: &'a CancellationToken,
    best_cost: u64,
    best_tree: Option<QueryTree>,
}

impl Search<'_, '_> {
    /// A fast search that finds a good, but not best plan.
    /// It works by making the "locally best" choice at each step.
    fn greedy(&mut self, mut current_tree: Option<QueryTree>, mut current_cost: u64) {
        let graph = self.graph;
        for alternatives in self.groups {
            let mut best_delta = u64::MAX;
            let mut best_candidate: Option<&Candidate> = None;

            // At each step, pick the candidate that adds the smallest cost to the current plan
            for candidate in alternatives {
                let delta = match current_tree.as_ref() {
                    Some(tree) => match candidate.added_cost(graph, &tree.root) {
                        Ok(added_cost) => added_cost,
                        Err(_) => continue,
                    },
                    None => candidate.cost,
                };

                if delta < best_delta {
                    best_delta = delta;
                    best_candidate = Some(candidate);
                }
            }

            let Some(candidate) = best_candidate else {
                return;
            };
            match current_tree.as_mut() {
                Some(tree) => {
                    if candidate
                        .merge_into(graph, Rc::make_mut(&mut tree.root))
                        .is_err()
                    {
                        return;
                    }
                }
                None => match candidate.tree(graph) {
                    Ok(tree) => current_tree = Some(tree),
                    Err(_) => return,
                },
            }
            current_cost += best_delta;
        }

        if let Some(tree) = current_tree {
            if current_cost < self.best_cost {
                self.best_cost = current_cost;
                self.best_tree = Some(tree);
            }
        }
    }

    /// Branch and bound (to prune early) depth-first search for the best combination.
    fn explore(
        &mut self,
        group_index: usize,
        tree_so_far: Option<QueryTree>,
        cost_so_far: u64,
    ) -> Result<(), QueryPlanError> {
        self.cancellation_token.bail_if_cancelled()?;

        // This is the most critical optimization.
        // If the current path's cost + the absolute best-case cost
        // for all remaining choices is already worse than our
        // best solution, we can abandon this entire search branch.
        if cost_so_far + self.min_remaining_costs[group_index] >= self.best_cost {
            return Ok(());
        }

        // If we've made a choice for every set of alternatives, we have a complete plan,
        // and it's the best one so far.
        let Some(alternatives) = self.groups.get(group_index) else {
            self.best_cost = cost_so_far;
            self.best_tree = tree_so_far;
            return Ok(());
        };

        let costs = Costs(self.graph);
        for candidate in alternatives {
            let candidate_tree = candidate.tree(self.graph)?;

            let added_cost = match tree_so_far.as_ref() {
                Some(tree) => costs.added_by_merge(&tree.root, &candidate_tree.root),
                None => costs.tree(&candidate_tree.root),
            };
            let next_cost = cost_so_far + added_cost;
            // If the next step alone is too expensive, skip it.
            if next_cost >= self.best_cost {
                continue;
            }

            let next_tree = match tree_so_far.as_ref() {
                Some(tree) => {
                    let mut merged = tree.clone();
                    Rc::make_mut(&mut merged.root).merge_nodes(&candidate_tree.root);
                    merged
                }
                None => candidate_tree,
            };

            self.explore(group_index + 1, Some(next_tree), next_cost)?;
        }

        Ok(())
    }
}

/// Where the node of `segment` is among the children of `node`, compared like
/// `QueryTreeNode::eq` does. A node made from a path has no context coordinate.
fn find_path_child(
    node: &QueryTreeNode,
    tail_node_index: NodeIndex,
    segment: &PathSegment,
    position: MutationFieldPosition,
) -> Option<usize> {
    node.children.iter().position(|child| {
        child.node_index == tail_node_index
            && child.edge_from_parent == Some(segment.edge_index)
            && child.selection_attributes == segment.selection_attributes
            && child.mutation_field_position == position
            && child.condition == segment.condition
            && child.context_coordinate().is_none()
    })
}

/// Merges the tree of one path into `root`, without building that tree first.
/// Same as merging `QueryTree::from_path`, which makes a node per segment just to find most
/// of them in `root` already and drop them.
/// Only for paths without `@fromContext` requirements, their trees merge into each other.
fn merge_path(
    graph: &Graph<'_>,
    root: &mut QueryTreeNode,
    segments: &[&PathSegment],
    mutation_field_position: MutationFieldPosition,
) -> Result<(), GraphError> {
    let mut node = root;
    for (index, segment) in segments.iter().enumerate() {
        let tail_node_index = graph.get_edge_tail(&segment.edge_index)?;
        // Only the first node of the path has the position
        let position = mutation_field_position.filter(|_| index == 0);

        let Some(existing) = find_path_child(node, tail_node_index, segment, position) else {
            let rest =
                QueryTreeNode::from_path_segment_sequences(graph, segments, index, position)?
                    .expect("a segment is left");
            node.children.push(rest);
            return Ok(());
        };

        let child = Rc::make_mut(&mut node.children[existing]);
        if let Some(requirement) = &segment.requirement_tree {
            merge_query_tree_node_list(&mut child.requirements, std::slice::from_ref(requirement));
        }
        node = child;
    }
    Ok(())
}

/// The cost model: what a tree costs, and what merging something into a tree adds to it.
#[derive(Clone, Copy)]
struct Costs<'a>(&'a Graph<'a>);

impl Costs<'_> {
    fn tree(self, node: &QueryTreeNode) -> u64 {
        let mut current_cost = FIELD_COST;

        for child in &node.children {
            if self.edge(child) > 0 {
                current_cost += CROSS_SUBGRAPH_COST;
            }

            current_cost += self.tree(child);
        }

        for requirement in &node.requirements {
            current_cost += CROSS_SUBGRAPH_COST;
            current_cost += self.tree(requirement);
        }

        for requirement in node.context_requirements() {
            current_cost += self.tree(requirement);
        }
        if !node.resolved_context_requirements().is_empty() {
            current_cost += CROSS_SUBGRAPH_COST;
        }

        current_cost
    }

    /// What getting to `node` from its parent costs.
    fn edge(self, node: &QueryTreeNode) -> u64 {
        if node.edge_from_parent.is_some_and(|edge_index| {
            matches!(
                self.0.edge(edge_index).expect("edge should exist"),
                Edge::SubgraphEntrypoint { .. }
            )
        }) {
            CROSS_SUBGRAPH_COST
        } else {
            0
        }
    }

    /// What the nodes of these segments cost, with the edge into the first one. For a whole path
    /// it's `tree` of its tree, without the root. No `@fromContext` here.
    fn of_segments(self, segments: &[&PathSegment]) -> Result<u64, GraphError> {
        let mut cost = 0;
        for segment in segments {
            cost += FIELD_COST;
            if matches!(
                self.0.edge(segment.edge_index)?,
                Edge::SubgraphEntrypoint { .. }
            ) {
                cost += CROSS_SUBGRAPH_COST;
            }
            if let Some(requirement) = &segment.requirement_tree {
                cost += CROSS_SUBGRAPH_COST + self.tree(requirement);
            }
        }
        Ok(cost)
    }

    /// What `merge_path` would add to the cost of `root`.
    fn added_by_path(
        self,
        root: &QueryTreeNode,
        segments: &[&PathSegment],
        mutation_field_position: MutationFieldPosition,
    ) -> Result<u64, GraphError> {
        let mut added_cost = 0;
        let mut node = root;
        for (index, segment) in segments.iter().enumerate() {
            let tail_node_index = self.0.get_edge_tail(&segment.edge_index)?;
            let position = mutation_field_position.filter(|_| index == 0);

            let Some(existing) = find_path_child(node, tail_node_index, segment, position) else {
                return Ok(added_cost + self.of_segments(&segments[index..])?);
            };

            let child = &node.children[existing];
            if let Some(requirement) = &segment.requirement_tree {
                added_cost += self.added_by_list(
                    &child.requirements,
                    std::slice::from_ref(requirement),
                    true,
                );
            }
            node = child;
        }
        Ok(added_cost)
    }

    /// How much `source` adds when merged into `target`.
    ///
    /// Only nodes missing from `target`, and their missing descendants and requirements,
    /// contribute additional cost. This avoids re-costing the entire merged tree while
    /// retaining the same cost as a full merge followed by `tree`.
    fn added_by_merge(self, target: &QueryTreeNode, source: &QueryTreeNode) -> u64 {
        // context trees merge like requirements, but `tree` charges no hop for them,
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
                    Some(target_node) => self.added_by_merge(target_node, source_node),
                    None => self.tree(source_node),
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

        self.added_by_list(&target.children, &source.children, false)
            + self.added_by_list(&target.requirements, &source.requirements, true)
            + context_cost
            + resolved_context_cost
    }

    fn added_by_list(
        self,
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
                    self.added_by_merge(target_node, source_node)
                } else if is_requirement {
                    CROSS_SUBGRAPH_COST + self.tree(source_node)
                } else {
                    self.edge(source_node) + self.tree(source_node)
                }
            })
            .sum()
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
        let costs = Costs(&graph);

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

        let current_cost = costs.tree(&target);
        let added_cost = costs.added_by_merge(&target, &source);
        let mut merged = target.clone();
        merged.merge_nodes(&source);

        assert_eq!(added_cost, costs.tree(&merged) - current_cost);
    }
}
