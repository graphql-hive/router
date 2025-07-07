use std::fmt::Write;

use super::query_tree_node::QueryTreeNode;
use crate::{
    graph::{error::GraphError, Graph},
    planner::{
        tree::query_tree_node::MutationFieldPosition,
        walker::{path::OperationPath, WalkContext},
    },
};
use bumpalo::collections::Vec as BumpVec;
use tracing::instrument;

#[derive(Debug, Copy, Clone)]
pub struct QueryTree<'bump> {
    pub root: &'bump QueryTreeNode<'bump>,
}

impl<'bump> QueryTree<'bump> {
    pub fn new(root_node: &'bump QueryTreeNode<'bump>) -> Self {
        QueryTree { root: root_node }
    }

    #[instrument(level = "trace",skip(ctx), fields(
        root_node = ctx.graph.pretty_print_node(&path.root_node)
    ))]
    pub fn from_path(
        ctx: &WalkContext<'bump>,
        path: &OperationPath<'bump>,
        mutation_field_position: MutationFieldPosition,
    ) -> Result<Self, GraphError> {
        let segments = path.get_segments();

        let root_node = QueryTreeNode::create_root_for_path_sequences(
            ctx,
            &path.root_node,
            &segments,
            mutation_field_position,
        )?;

        Ok(QueryTree::new(root_node))
    }

    #[instrument(level = "trace",skip_all, fields(
      tree_count = trees.len()
    ))]
    pub fn merge_trees(
        ctx: &WalkContext<'bump>,
        mut trees: BumpVec<'bump, QueryTree<'bump>>,
    ) -> QueryTree<'bump> {
        if trees.is_empty() {
            panic!("merge_trees cannot be called with an empty Vec<QueryTree>.");
        }

        // Pop the first tree to use as the initial accumulator.
        let mut accumulator = trees.pop().unwrap();

        // Iterate over the remaining trees.
        for item in trees {
            // We need a mutable clone of the accumulator's root to merge into.
            // `clone_in` creates a deep copy within the arena.
            let mut new_root = accumulator.root.clone_in(ctx);

            // Merge the other tree's root into our mutable clone.
            new_root.merge_nodes(ctx, item.root);

            // Allocate the newly merged node in the arena and update the accumulator's root.
            accumulator.root = ctx.arena.alloc(new_root);
        }

        accumulator
    }

    pub fn pretty_print(&self, graph: &Graph) -> Result<String, std::fmt::Error> {
        let mut result = String::new();
        let root_node = graph.node(self.root.node_index).unwrap();
        write!(result, "{}", root_node)?;

        for step in self.root.children.iter() {
            write!(result, "{}", step.pretty_print(graph, 1)?)?;
        }

        Ok(result)
    }
}
