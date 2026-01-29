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
//!     each set of `Alternatives`.
//!
//! 2.  Before the main search, a fast Greedy Algorithm (`find_initial_plan`)
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
//!     potentially expensive work of building and costing a `QueryTree` is only ever
//!     done if a candidate is actually evaluated, and never more
//!     than once.

use lazy_init::LazyTransform;
use std::{cell::OnceCell, rc::Rc};

use crate::{
    graph::{edge::Edge, error::GraphError, Graph},
    planner::{
        error::QueryPlanError,
        tree::{
            query_tree::QueryTree,
            query_tree_node::{MutationFieldPosition, QueryTreeNode},
        },
        walker::{path::OperationPath, ResolvedOperation},
    },
    state::supergraph_state::OperationKind,
    utils::cancellation::CancellationToken,
};

type PathAndPosition = (OperationPath, MutationFieldPosition);
type QueryTreeResult = Result<QueryTree, GraphError>;
type LazyQueryTree = LazyTransform<PathAndPosition, QueryTreeResult>;

/// The high-penalty cost for crossing a subgraph boundary or satisfying a requirement.
/// This is the primary driver of the optimization, encouraging plans with fewer subgraphs.
const CROSS_SUBGRAPH_COST: u64 = 1000;
const FIELD_COST: u64 = 1;

/// A lazily-evaluated, potential piece of the final query plan.
/// It represents one of many possible ways to resolve a part of the query.
#[derive(Clone)]
struct Candidate {
    /// The actual `QueryTree` for this candiate, computed only when first needed.
    tree: LazyQueryTree,
    /// The cached cost of this tree, computed only once.
    cost: OnceCell<u64>,
}

/// Represents a set of alternative Candidates to satisfy a single leaf in the query.
/// The final plan must choose exactly one fragment from each ChoiceGroup.
type Alternatives = Vec<Candidate>;

impl Candidate {
    fn new(path: OperationPath, mutation_pos: MutationFieldPosition) -> Self {
        Self {
            tree: LazyTransform::new((path, mutation_pos)),
            cost: OnceCell::new(),
        }
    }

    #[inline]
    fn get_tree(&self, graph: &Graph) -> Result<QueryTree, QueryPlanError> {
        Ok(self
            .tree
            .get_or_create(|(p, mp)| QueryTree::from_path(graph, &p, mp))
            .clone()?)
    }

    #[inline]
    fn get_cost(&self, graph: &Graph) -> Result<u64, QueryPlanError> {
        if let Some(v) = self.cost.get() {
            return Ok(*v);
        }
        let tree = self.get_tree(graph)?;
        let cost = calculate_cost_of_tree(graph, &tree.root);
        let _ = self.cost.set(cost);
        Ok(cost)
    }
}

/// Each `Alternatives` group represents a set of possible ways to satisfy one leaf of the query.
fn prepare_alternatives(operation: ResolvedOperation) -> Vec<Alternatives> {
    let is_mutation = matches!(operation.operation_kind, OperationKind::Mutation);
    let mut per_leaf_alternatives_asc: Vec<Alternatives> = Vec::new();

    for (index, root_field_options) in operation.root_field_groups.into_iter().enumerate() {
        let mutation_field_position: MutationFieldPosition = is_mutation.then_some(index);

        let leaf_alternatives: Vec<Alternatives> = root_field_options
            .into_iter()
            .map(|paths_to_leaf| {
                paths_to_leaf
                    .into_iter()
                    .map(|op| Candidate::new(op, mutation_field_position))
                    .collect::<Alternatives>()
            })
            .collect();

        per_leaf_alternatives_asc.extend(leaf_alternatives);
    }

    // Sort alternatives by length in ascending order.
    per_leaf_alternatives_asc.sort_by_key(|alternatives| alternatives.len());

    per_leaf_alternatives_asc
}

fn calculate_min_remaining_costs(
    graph: &Graph,
    per_leaf_alternatives_asc: &[Alternatives],
) -> Result<Vec<u64>, QueryPlanError> {
    // Pre-compute a lower bound for pruning.
    // Finds the absolute best-case cost for each set of alternatives.
    let best_case_cost_per_leaf = per_leaf_alternatives_asc
        .iter()
        .map(|alternatives| {
            alternatives
                .iter()
                .map(|candidate| candidate.get_cost(graph))
                .try_fold(u64::MAX, |acc, cost_result| {
                    Ok::<u64, QueryPlanError>(acc.min(cost_result?))
                })
        })
        .collect::<Result<Vec<u64>, _>>()?;

    // Pre-compute a "suffix lower bound" (LB) to enable effective pruning.
    // `min_remaining_costs[i]` stores an optimistic "best-case" cost for completing the
    // plan from alternatives `i` to the end.
    // In the `explore_plan_combinations`, we can then check:
    // `cost_so_far + min_remaining_costs[depth] >= best_cost_so_far`.
    // If this is true, we know the current path is a dead end and can be abandoned.
    let mut min_remaining_costs = vec![0; per_leaf_alternatives_asc.len() + 1];
    for i in (0..per_leaf_alternatives_asc.len()).rev() {
        min_remaining_costs[i] = min_remaining_costs[i + 1] + best_case_cost_per_leaf[i];
    }
    Ok(min_remaining_costs)
}

fn sort_candidates_by_cost(graph: &Graph, per_leaf_alternatives_asc: &mut [Alternatives]) {
    // Within each set of alternatives, sort choices by their cost.
    // Trying cheaper options first
    // is more likely to lead to better solutions sooner.
    for paths in per_leaf_alternatives_asc {
        paths.sort_by_key(|c| c.get_cost(graph).unwrap_or(u64::MAX));
    }
}

pub fn find_best_combination(
    graph: &Graph,
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

    let mut per_leaf_alternatives_asc = prepare_alternatives(operation);
    if per_leaf_alternatives_asc.is_empty() {
        return Err(QueryPlanError::EmptyPlan);
    }

    let min_remaining_costs = calculate_min_remaining_costs(graph, &per_leaf_alternatives_asc)?;
    sort_candidates_by_cost(graph, &mut per_leaf_alternatives_asc);

    let mut best_cost = u64::MAX;
    let mut best_tree: Option<QueryTree> = None;

    // Runs a fast, greedy search to find a good-enough initial solution.
    // A strong initial `best_cost` is critical for effective pruning.
    if let Some((cost, tree)) = find_initial_plan(graph, &per_leaf_alternatives_asc) {
        if cost < best_cost {
            best_cost = cost;
            best_tree = Some(tree);
        }
    }

    let mut state = ExplorationState {
        best_cost,
        best_tree,
    };

    // Performs the search for the optimal solution
    explore_plan_combinations(
        graph,
        &per_leaf_alternatives_asc,
        0,
        None,
        0,
        &min_remaining_costs,
        cancellation_token,
        &mut state,
    )?;

    state.best_tree.ok_or(QueryPlanError::EmptyPlan)
}

/// A fast search that finds a good, but not best plan.
/// It works by making the "locally best" choice at each step.
fn find_initial_plan(
    graph: &Graph,
    alternatives_list: &[Alternatives],
) -> Option<(u64, QueryTree)> {
    let mut current_tree: Option<QueryTree> = None;
    let mut current_cost: u64 = 0;

    for alternatives in alternatives_list {
        let mut best_delta = u64::MAX;
        let mut best_next: Option<(u64, QueryTree)> = None;

        // At each step, pick the candidate that adds the smallest cost to the current plan
        for candidate in alternatives {
            let cand_tree = match candidate.get_tree(graph) {
                Ok(t) => t,
                Err(_) => continue,
            };
            let next_tree = match current_tree.as_ref() {
                Some(t) => {
                    let mut merged = t.clone();
                    Rc::make_mut(&mut merged.root).merge_nodes(&cand_tree.root);
                    merged
                }
                None => cand_tree.clone(),
            };
            let next_cost = calculate_cost_of_tree(graph, &next_tree.root);
            let delta = next_cost.saturating_sub(current_cost);
            if delta < best_delta {
                best_delta = delta;
                best_next = Some((next_cost, next_tree));
            }
        }

        if let Some((next_cost, next_tree)) = best_next {
            current_cost = next_cost;
            current_tree = Some(next_tree);
        } else {
            return None;
        }
    }

    current_tree.map(|t| (current_cost, t))
}

struct ExplorationState {
    best_cost: u64,
    best_tree: Option<QueryTree>,
}

#[allow(clippy::too_many_arguments)]
/// Performs branch-and-bound (to prune early) depth-first search to find the best combination
fn explore_plan_combinations(
    graph: &Graph,
    groups: &[Alternatives],
    group_index: usize,
    tree_so_far: Option<QueryTree>,
    cost_so_far: u64,
    min_remaining_costs: &[u64],
    cancellation_token: &CancellationToken,
    state: &mut ExplorationState,
) -> Result<(), QueryPlanError> {
    cancellation_token.bail_if_cancelled()?;

    // This is the most critical optimization.
    // If the current path's cost + the absolute best-case cost
    // for all remaining choices is already worse than our
    // best solution, we can abandon this entire search branch.
    if cost_so_far + min_remaining_costs[group_index] >= state.best_cost {
        return Ok(());
    }

    // If we've made a choice for every set of alternatives,
    // we have a complete plan.
    // If it's the best one we've seen, we save it.
    if group_index == groups.len() {
        if cost_so_far < state.best_cost {
            state.best_cost = cost_so_far;
            state.best_tree = tree_so_far;
        }
        return Ok(());
    }

    // Explore each possible candidate for the current set of alternatives.
    for cand in groups[group_index].iter() {
        let cand_tree = cand.get_tree(graph)?;
        let next_tree = match tree_so_far.as_ref() {
            Some(t) => {
                let mut merged = t.clone();
                Rc::make_mut(&mut merged.root).merge_nodes(&cand_tree.root);
                merged
            }
            None => cand_tree.clone(),
        };

        let next_cost = calculate_cost_of_tree(graph, &next_tree.root);
        // If the next step alone is too expensive, skip it.
        if next_cost >= state.best_cost {
            continue;
        }

        explore_plan_combinations(
            graph,
            groups,
            group_index + 1,
            Some(next_tree),
            next_cost,
            min_remaining_costs,
            cancellation_token,
            state,
        )?;
    }

    Ok(())
}

#[inline(always)]
fn calculate_cost_of_tree(graph: &Graph, node: &QueryTreeNode) -> u64 {
    let mut current_cost = FIELD_COST;

    for child in &node.children {
        if child.edge_from_parent.is_some_and(|edge_index| {
            matches!(
                graph.edge(edge_index).expect("edge should exist"),
                Edge::SubgraphEntrypoint { .. }
            )
        }) {
            current_cost += CROSS_SUBGRAPH_COST;
        }

        current_cost += calculate_cost_of_tree(graph, child);
    }

    for requirement in &node.requirements {
        current_cost += CROSS_SUBGRAPH_COST;
        current_cost += calculate_cost_of_tree(graph, requirement);
    }

    current_cost
}
