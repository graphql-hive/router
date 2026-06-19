//! # Best Plan Finder
//!
//! This module finds the best execution plan for a GraphQL query.
//!
//! A single query can be resolved by many different combinations of subgraphs.
//! For example, a query with 3 fields might be satisfied by:
//! - All fields from subgraph A (1 subgraph, good)
//! - Field 1 from A, fields 2+3 from B (2 subgraphs, worse)
//! - Each field from a different subgraph (3 subgraphs, worst)
//!
//! More subgraph crossings mean more network requests and slower responses.
//! This module picks the combination with the lowest "cost".
//!
//! 1. Group all possible candidates for each field into "alternatives".
//!    Each candidate is one way to resolve a field.
//!
//! 2. If a field has only one candidate, merge it right away.
//!    This reduces the number of candidates we need to consider later.
//!
//! 3. For each remaining group of alternatives, pick the candidate that adds
//!    the lowest cost to the plan we already have.

use lazy_init::LazyTransform;
use std::rc::Rc;

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

type PathAndPosition<'graph> = (OperationPath<'graph>, MutationFieldPosition);
type QueryTreeResult = Result<QueryTree, GraphError>;
type LazyQueryTree<'graph> = LazyTransform<PathAndPosition<'graph>, QueryTreeResult>;

/// The high-penalty cost for crossing a subgraph boundary or satisfying a requirement.
/// This is the primary driver of the optimization, encouraging plans with fewer subgraphs.
const CROSS_SUBGRAPH_COST: u64 = 1000;
const FIELD_COST: u64 = 1;

/// A lazily-evaluated, potential piece of the final query plan.
/// It represents one of many possible ways to resolve a part of the query.
#[derive(Clone)]
struct Candidate<'graph> {
    /// The actual `QueryTree` for this candiate, computed only when first needed.
    tree: LazyQueryTree<'graph>,
}

/// A group of alternatives - all possible ways to resolve one field.
/// The final plan must pick exactly one Candidate from each group.
type Alternatives<'graph> = Vec<Candidate<'graph>>;

impl<'graph> Candidate<'graph> {
    fn new(path: OperationPath<'graph>, mutation_pos: MutationFieldPosition) -> Self {
        Self {
            tree: LazyTransform::new((path, mutation_pos)),
        }
    }

    #[inline]
    fn get_tree(&self, graph: &Graph<'_>) -> Result<QueryTree, QueryPlanError> {
        Ok(self
            .tree
            .get_or_create(|(p, mp)| QueryTree::from_path(graph, &p, mp))
            .clone()?)
    }
}

/// Build the list of alternatives for each field in the query.
/// Returns a list where each entry is all the ways to resolve one field.
fn prepare_alternatives<'graph>(operation: ResolvedOperation<'graph>) -> Vec<Alternatives<'graph>> {
    let is_mutation = matches!(operation.operation_kind, OperationKind::Mutation);

    let mut per_leaf_alternatives: Vec<Alternatives<'graph>> = operation
        .root_field_groups
        .into_iter()
        .enumerate()
        .flat_map(|(index, root_field_options)| {
            // For mutations, we need to track the position of each field
            // because mutation fields must be executed in order.
            let mutation_field_position: MutationFieldPosition = is_mutation.then_some(index);

            root_field_options.into_iter().map(move |paths_to_leaf| {
                paths_to_leaf
                    .into_iter()
                    .map(|op| Candidate::new(op, mutation_field_position))
                    .collect::<Alternatives>()
            })
        })
        .collect();

    // Sort by number of alternatives (fewest first).
    // Fields with only 1 candidate are fixed and easy.
    // Processing them first means fewer candidates to consider later.
    per_leaf_alternatives.sort_by_key(|alternatives| alternatives.len());

    per_leaf_alternatives
}

/// Merge fields that have only one possible resolution.
///
/// Returns:
/// - The merged base tree, if any singletons were found.
/// - The remaining alternatives, which still have more than one candidate.
fn merge_singleton_alternatives<'graph>(
    graph: &Graph<'_>,
    per_leaf_alternatives: Vec<Alternatives<'graph>>,
) -> Result<(Option<QueryTree>, Vec<Alternatives<'graph>>), QueryPlanError> {
    // Split into singletons (1 candidate) and non-singletons (2+ candidates).
    let (singletons, remaining): (Vec<_>, Vec<_>) = per_leaf_alternatives
        .into_iter()
        .partition(|a| a.len() == 1);

    // Merge all singletons into one base tree.
    let mut base_tree: Option<QueryTree> = None;
    for alternatives in singletons {
        let candidate = alternatives
            .into_iter()
            .next()
            .expect("singleton has one candidate");
        let candidate_tree = candidate.get_tree(graph)?;
        match base_tree.as_mut() {
            // Merge into the existing base tree.
            Some(tree) => Rc::make_mut(&mut tree.root).merge_nodes(&candidate_tree.root),
            // This is the first singleton - it becomes the base tree.
            None => base_tree = Some(candidate_tree),
        }
    }

    Ok((base_tree, remaining))
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

    let per_leaf_alternatives = prepare_alternatives(operation);
    if per_leaf_alternatives.is_empty() {
        return Err(QueryPlanError::EmptyPlan);
    }

    // Merge fields with only one candidate
    let (base_tree, per_leaf_alternatives) =
        merge_singleton_alternatives(graph, per_leaf_alternatives)?;

    // If all fields were singletons, we are done.
    if per_leaf_alternatives.is_empty() {
        return base_tree.ok_or(QueryPlanError::EmptyPlan);
    }

    let mut current_tree = base_tree;
    let mut current_cost = current_tree
        .as_ref()
        .map(|tree| calculate_cost_of_tree(graph, &tree.root))
        .unwrap_or(0);

    // For each group, choose the candidate that adds the lowest cost.
    //
    // This is fast because we do not build a temporary merged tree for every
    // candidate. That is too expensive for large queries.
    // Instead, we compare the current tree with the candidate tree
    // and calculate only the cost that is missing from the current tree.
    for alternatives in &per_leaf_alternatives {
        cancellation_token.bail_if_cancelled()?;

        let mut best_next: Option<(u64, QueryTree)> = None;

        for candidate in alternatives {
            let candidate_tree = match candidate.get_tree(graph) {
                Ok(tree) => tree,
                Err(_) => continue,
            };

            let added_cost = match current_tree.as_ref() {
                Some(tree) => {
                    calculate_added_cost_of_merge(graph, &tree.root, &candidate_tree.root)
                }
                None => calculate_cost_of_tree(graph, &candidate_tree.root),
            };
            let next_cost = current_cost + added_cost;

            if best_next
                .as_ref()
                .is_none_or(|(best_cost, _)| next_cost < *best_cost)
            {
                best_next = Some((next_cost, candidate_tree));
            }
        }

        let (next_cost, next_tree) = best_next.ok_or(QueryPlanError::EmptyPlan)?;

        // Merge only the selected candidate. Rejected candidates were only read.
        match current_tree.as_mut() {
            Some(tree) => Rc::make_mut(&mut tree.root).merge_nodes(&next_tree.root),
            None => current_tree = Some(next_tree),
        }
        current_cost = next_cost;
    }

    current_tree.ok_or(QueryPlanError::EmptyPlan)
}

/// Calculate how much cost `source` would add to `target`.
///
/// `target` is the query tree we already selected.
/// `source` is a candidate tree we may select next.
///
/// We only count nodes that are missing from `target`. If a node already exists,
/// we go deeper and check its children and requirements. This gives the same
/// cost as a full merge followed by a full cost calculation, but it avoids the
/// expensive temporary merge.
fn calculate_added_cost_of_merge(
    graph: &Graph<'_>,
    target: &QueryTreeNode,
    source: &QueryTreeNode,
) -> u64 {
    calculate_added_cost_for_node_list(graph, &target.children, &source.children, false)
        + calculate_added_cost_for_node_list(
            graph,
            &target.requirements,
            &source.requirements,
            true,
        )
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
            // If the node already exists, only its missing nested parts add cost.
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

/// Calculate the total cost of a query tree node and all its children.
#[inline(always)]
fn calculate_cost_of_tree(graph: &Graph<'_>, node: &QueryTreeNode) -> u64 {
    let mut current_cost = FIELD_COST;

    // Add cost for each child node
    for child in &node.children {
        if edge_cost(graph, child) > 0 {
            // If this child crosses into a different subgraph
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
