use std::{fmt::Write, sync::Arc};

use tracing::instrument;

use crate::{
    graph::{error::GraphError, Graph},
    planner::{tree::query_tree_node::MutationFieldPosition, walker::path::OperationPath},
};

use super::query_tree_node::QueryTreeNode;

#[derive(Debug, Clone)]
pub struct QueryTree {
    pub root: Arc<QueryTreeNode>,
}

impl QueryTree {
    fn new(root_node: QueryTreeNode) -> Self {
        QueryTree {
            root: Arc::new(root_node),
        }
    }

    #[instrument(level = "trace",skip(graph), fields(
        root_node = graph.pretty_print_node(&path.root_node)
    ))]
    pub fn from_path(
        graph: &Graph,
        path: &OperationPath,
        mutation_field_position: MutationFieldPosition,
    ) -> Result<Self, GraphError> {
        let segments = path.get_segments();

        let root_node = QueryTreeNode::create_root_for_path_sequences(
            graph,
            &path.root_node,
            &segments,
            mutation_field_position,
        )?;

        Ok(QueryTree::new(root_node))
    }

    #[instrument(level = "trace",skip_all, fields(
      tree_count = trees.len()
    ))]
    pub fn merge_trees(trees: Vec<QueryTree>) -> QueryTree {
        if trees.is_empty() {
            panic!("merge_trees cannot be called with an empty Vec<QueryTree>.");
        }

        let mut iter = trees.into_iter();
        // `unwrap()` is safe here because we've just checked that `trees` is not empty.
        let mut accumulator = iter.next().unwrap();

        // Iterate over the remaining trees in the iterator.
        for item in iter {
            let accumulator_root_mut = Arc::make_mut(&mut accumulator.root);
            accumulator_root_mut.merge_nodes(&item.root);
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
