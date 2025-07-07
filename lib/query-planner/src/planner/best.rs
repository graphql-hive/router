use lazy_init::LazyTransform;

use crate::{
    graph::{edge::Edge, error::GraphError, Graph},
    planner::{
        error::QueryPlanError,
        tree::{
            query_tree::QueryTree,
            query_tree_node::{MutationFieldPosition, QueryTreeNode},
        },
        walker::{path::OperationPath, ResolvedOperation, WalkContext},
    },
    state::supergraph_state::OperationKind,
};
use bumpalo::collections::Vec as BumpVec;

// Type aliases for clarity and brevity, now using bump-allocated structures.
type PathAndPosition<'bump> = (OperationPath<'bump>, MutationFieldPosition);
type QueryTreeResult<'bump> = Result<QueryTree<'bump>, GraphError>;
type LazyQueryTree<'bump> = LazyTransform<PathAndPosition<'bump>, QueryTreeResult<'bump>>;
type LazyQueryTreeList<'bump> = BumpVec<'bump, LazyQueryTree<'bump>>;
type BestLazyTreesPerLeaf<'bump> = BumpVec<'bump, LazyQueryTreeList<'bump>>;

/// Finds the best combination of paths to leafs.
/// It compares all possible paths to one leaf, with all possible paths to another leaf.
/// It does not compare all, it tries to be smart, we will see in practice if it really is :)
pub fn find_best_combination<'bump>(
    ctx: &WalkContext<'bump>,
    operation: ResolvedOperation<'bump>,
) -> Result<QueryTree<'bump>, QueryPlanError> {
    if operation.root_field_groups.is_empty()
        || operation
            .root_field_groups
            .iter()
            .any(|paths_to_leafs| paths_to_leafs.iter().any(|p| p.is_empty()))
    {
        return Err(QueryPlanError::EmptyPlan);
    }

    let is_mutation = matches!(operation.operation_kind, OperationKind::Mutation);

    let mut best_trees_per_leaf: BestLazyTreesPerLeaf<'bump> = BumpVec::new_in(ctx.arena);

    for (index, root_field_options) in operation.root_field_groups.into_iter().enumerate() {
        let mut mutation_field_position: MutationFieldPosition = None;
        if is_mutation {
            mutation_field_position = Some(index);
        }

        let leafs = BumpVec::from_iter_in(
            root_field_options.into_iter().map(|paths_to_leaf| {
                BumpVec::from_iter_in(
                    paths_to_leaf
                        .into_iter()
                        .map(|op| LazyTransform::new((op, mutation_field_position))),
                    ctx.arena,
                )
            }),
            ctx.arena,
        );

        best_trees_per_leaf.extend(leafs);
    }

    // Sorts the groups of paths by how many alternative paths they have.
    // We process leafs with fewer alternatives first. This can help speed up
    // finding the best overall combination.
    // It can help find a good candidate faster, which then
    // allows the algorithm to more effectively
    // prune away more complex options later in the search.
    best_trees_per_leaf.sort_by_key(|paths| paths.len());

    let mut min_overall_cost = u64::MAX;
    let mut final_best_tree: Option<QueryTree<'bump>> = None;

    // Start from the first leaf with the least amount of possible paths
    explore_tree_combinations(
        ctx,
        &best_trees_per_leaf,
        0,
        None,
        &mut min_overall_cost,
        &mut final_best_tree,
    )?;

    final_best_tree.ok_or(QueryPlanError::EmptyPlan)
}

fn explore_tree_combinations<'bump>(
    ctx: &WalkContext<'bump>,
    best_trees_per_leaf: &BestLazyTreesPerLeaf<'bump>,
    // Index of the outer vec of best_trees_per_leaf
    current_leaf_index: usize,
    tree_so_far: Option<QueryTree<'bump>>,
    min_cost_so_far: &mut u64,
    best_tree_so_far: &mut Option<QueryTree<'bump>>,
) -> Result<(), QueryPlanError> {
    // Looks like all leafs has been processed
    if current_leaf_index == best_trees_per_leaf.len() {
        if let Some(final_tree) = tree_so_far {
            // Calculates the final cost of the full tree
            let cost = calculate_cost_of_tree(ctx.graph, final_tree.root);
            if cost < *min_cost_so_far {
                *min_cost_so_far = cost;
                *best_tree_so_far = Some(final_tree);
            }
        }
        return Ok(());
    }

    best_trees_per_leaf[current_leaf_index]
        .iter()
        .try_fold((), |(), tree_candidate| {
            // The closure captures `ctx`, allowing `from_path` to use the arena.
            // The closure captures `ctx`, allowing `from_path` to use the arena.
            let current_tree = tree_candidate
                .get_or_create(|(path, mutation_index)| {
                    // The `path` is a reference to the OperationPath stored in the LazyTransform.
                    // The `mutation_index` is a `&Option<usize>`, so we dereference it to get the `Option<usize>`.
                    QueryTree::from_path(ctx, &path, mutation_index)
                })
                .clone()?;

            // Merges the current tree with the tree we built so far
            let next_tree = match tree_so_far {
                Some(tree) => {
                    // Since QueryTree is now Copy, and its root is an immutable reference,
                    // we need to perform a deep clone to get a mutable version for merging.
                    let mut new_root = tree.root.clone_in(ctx);
                    new_root.merge_nodes(ctx, current_tree.root);

                    // Create a new QueryTree pointing to the new, merged root in the arena.
                    let new_root_ref = ctx.arena.alloc(new_root);
                    QueryTree::new(new_root_ref)
                }
                None => current_tree,
            };

            // If the cost of the tree so far is already greater than or equal to
            // the minimum cost found, skip exploring
            let cost = calculate_cost_of_tree(ctx.graph, next_tree.root);
            if cost >= *min_cost_so_far {
                return Ok(());
            }

            // Go to the next leaf
            explore_tree_combinations(
                ctx,
                best_trees_per_leaf,
                current_leaf_index + 1,
                Some(next_tree),
                min_cost_so_far,
                best_tree_so_far,
            )
        })
}

fn calculate_cost_of_tree<'bump>(graph: &Graph, node: &QueryTreeNode<'bump>) -> u64 {
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
