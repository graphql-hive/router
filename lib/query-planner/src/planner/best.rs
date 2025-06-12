use std::sync::Arc;

use lazy_init::LazyTransform;

use crate::{
    graph::{edge::Edge, error::GraphError, Graph},
    planner::{
        error::QueryPlanError,
        tree::{query_tree::QueryTree, query_tree_node::QueryTreeNode},
        walker::path::OperationPath,
    },
};

/// Finds the best combination of paths to leafs.
/// It compares all possible paths to one leaf, with all possible paths to another leaf.
/// It does not compare all, it tries to be smart, we will see in practice if it really is :)
pub fn find_best_combination(
    graph: &Graph,
    mut best_paths_per_leaf: Vec<Vec<OperationPath>>,
) -> Result<QueryTree, QueryPlanError> {
    if best_paths_per_leaf.is_empty() || best_paths_per_leaf.iter().any(Vec::is_empty) {
        return Err(QueryPlanError::EmptyPlan);
    }

    // Sorts the groups of paths by how many alternative paths they have.
    // We process leafs with fewer alternatives first. This can help speed up
    // finding the best overall combination.
    // It can help find a good candidate faster, which then
    // allows the algorithm to more effectively
    // prune away more complex options later in the search.
    best_paths_per_leaf.sort_by_key(|paths| paths.len());

    let best_trees_per_leaf: Vec<_> = best_paths_per_leaf
        // One thing I did not know is that `Vec` when used with `.into_iter()`
        // produces `ExactSizeIterator` so `map` preserves the exact size property.
        // Meaning it's the same as a for loop + Vec::with_capacity(vec.len())`
        .into_iter()
        .map(|paths_vec| paths_vec.into_iter().map(LazyTransform::new).collect())
        .collect();

    let mut min_overall_cost = u64::MAX;
    let mut final_best_tree: Option<QueryTree> = None;

    // Start from the first leaf with the least amount of possible paths
    explore_tree_combinations(
        graph,
        &best_trees_per_leaf,
        0,
        None,
        &mut min_overall_cost,
        &mut final_best_tree,
    )?;

    final_best_tree.ok_or(QueryPlanError::EmptyPlan)
}

fn explore_tree_combinations(
    graph: &Graph,
    best_trees_per_leaf: &Vec<Vec<LazyTransform<OperationPath, Result<QueryTree, GraphError>>>>,
    // Index of the outer vec of best_trees_per_leaf
    current_leaf_index: usize,
    tree_so_far: Option<QueryTree>,
    min_cost_so_far: &mut u64,
    best_tree_so_far: &mut Option<QueryTree>,
) -> Result<(), QueryPlanError> {
    // Looks like all leafs has been processed
    if current_leaf_index == best_trees_per_leaf.len() {
        if let Some(final_tree) = tree_so_far {
            // Calculates the final cost of the full tree
            let cost = calculate_cost_of_tree(graph, &final_tree.root);
            if &cost < min_cost_so_far {
                *min_cost_so_far = cost;
                *best_tree_so_far = Some(final_tree);
            }
        }
        return Ok(());
    }

    best_trees_per_leaf[current_leaf_index]
        .iter()
        .try_fold((), |(), tree_candidate| {
            let current_tree = tree_candidate
                .get_or_create(|path| QueryTree::from_path(graph, &path))
                .clone()?;

            // Merges the current tree with the tree we built so far
            let next_tree = match tree_so_far {
                Some(ref tree) => {
                    let mut new_tree = tree.clone();
                    // `root` is Arc<QueryTreeNode`.
                    // We perform clone-on-write here
                    Arc::make_mut(&mut new_tree.root).merge_nodes(&current_tree.root);
                    new_tree
                }
                None => current_tree.clone(),
            };

            // If the cost of the tree so far is already greater than or equal to
            // the minimum cost found, skip exploring
            if &calculate_cost_of_tree(graph, &next_tree.root) >= min_cost_so_far {
                return Ok(());
            }

            // Go to the next leaf
            explore_tree_combinations(
                graph,
                best_trees_per_leaf,
                current_leaf_index + 1,
                Some(next_tree),
                min_cost_so_far,
                best_tree_so_far,
            )
        })
}

fn calculate_cost_of_tree(graph: &Graph, node: &QueryTreeNode) -> u64 {
    let mut current_cost: u64 = 1;

    for child in &node.children {
        if child.edge_from_parent.is_some_and(|edge_index| {
            matches!(
                graph.edge(edge_index).expect("to find an edge"),
                Edge::SubgraphEntrypoint { .. }
            )
        }) {
            // We do this to make sure two subgraph entries result in the same cost as two entity moves.
            // SubgraphEntrypoint is not part of `node.requirements` meaning we would treat it as +1.
            current_cost += 1000;
        }

        current_cost += calculate_cost_of_tree(graph, child);
    }

    for req in &node.requirements {
        current_cost += 1000;
        current_cost += calculate_cost_of_tree(graph, req);
    }

    current_cost
}
