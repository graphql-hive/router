use std::fmt::Write;

use tracing::instrument;

use crate::{
    graph::{error::GraphError, Graph},
    planner::walker::path::OperationPath,
};

use super::query_tree_node::QueryTreeNode;

#[derive(Debug, Clone)]
pub struct QueryTree {
    pub root: QueryTreeNode,
}

impl QueryTree {
    fn new(root: QueryTreeNode) -> Self {
        QueryTree { root }
    }

    #[instrument(skip(graph), fields(
        root_node = graph.pretty_print_node(&path.root_node)
    ))]
    pub fn from_path(graph: &Graph, path: &OperationPath) -> Result<Self, GraphError> {
        let segments = path.get_segments();

        let root_node =
            QueryTreeNode::create_root_for_path_sequences(graph, &path.root_node, &segments)?;

        Ok(QueryTree::new(root_node))
    }

    #[instrument(skip_all, fields(
      tree_count = trees.len()
    ))]
    pub fn merge_trees(mut trees: Vec<QueryTree>) -> QueryTree {
        let mut accumulator = trees.remove(0);

        for item in trees {
            accumulator.root = accumulator.root.merge_nodes(&item.root);
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
