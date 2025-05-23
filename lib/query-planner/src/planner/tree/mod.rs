pub mod query_tree;
pub(crate) mod query_tree_node;

use super::walker::path::OperationPath;
use crate::graph::{error::GraphError, Graph};
use query_tree::QueryTree;

pub fn paths_to_trees(
    graph: &Graph,
    paths: &[Vec<OperationPath>],
) -> Result<Vec<QueryTree>, GraphError> {
    let mut trees: Vec<QueryTree> = vec![];

    for paths in paths {
        let tree = QueryTree::from_path(graph, &paths[0])?;
        trees.push(tree);
    }

    Ok(trees)
}
