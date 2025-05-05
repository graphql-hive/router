use petgraph::graph::{EdgeIndex, NodeIndex};
use std::fmt::Write;
use tracing::{debug, instrument};

use crate::{
    graph::{edge::Edge, Graph},
    planner::walker::path::OperationPath,
};

use super::query_tree::QueryTree;

#[derive(Debug, Clone)]
pub struct QueryTreeNode {
    /// The underlying graph node this query tree node corresponds to
    pub node_index: NodeIndex,
    /// The edge from the parent QueryTreeNode that led to this node (null for root)
    pub edge_from_parent: Option<EdgeIndex>,
    /// Nodes required to execute the move
    pub requirements: Vec<QueryTreeNode>,
    pub children: Vec<QueryTreeNode>,
}

impl QueryTreeNode {
    pub fn new(node_index: &NodeIndex, edge_from_parent: Option<&EdgeIndex>) -> Self {
        QueryTreeNode {
            node_index: *node_index,
            edge_from_parent: edge_from_parent.cloned(),
            requirements: Vec::new(),
            children: Vec::new(),
        }
    }

    pub fn new_root(node_index: &NodeIndex) -> Self {
        QueryTreeNode::new(node_index, None)
    }

    pub fn merge_nodes(mut self, mut other: QueryTreeNode) -> Self {
        self.requirements.append(&mut other.requirements);
        self.children.append(&mut other.children);

        self
    }

    // #[instrument(skip(graph), fields(
    //   paths = paths.iter().map(|path| path.pretty_print(graph)).collect::<Vec<String>>().join(", ")
    // ))]
    pub fn from_paths(graph: &Graph, paths: &[OperationPath]) -> Option<Self> {
        if paths.is_empty() {
            return None;
        }

        let mut trees = paths
            .iter()
            .map(|path| QueryTree::from_path(graph, path))
            .collect::<Vec<_>>();

        if trees.len() == 1 {
            return Some(trees.remove(0).root);
        }

        Some(QueryTree::merge_trees(trees).root)
    }

    fn from_path_segment_sequences(
        _graph: &Graph,
        _edges: &[EdgeIndex],
        _requirements_tree: &[Option<&QueryTreeNode>],
        _current_index: usize,
    ) -> Option<QueryTreeNode> {
        None
    }

    #[instrument(skip(graph))]
    pub fn create_root_for_path_sequences(
        graph: &Graph,
        root_node_index: &NodeIndex,
        edges: &Vec<EdgeIndex>,
        requirements_tree: &Vec<Option<&QueryTreeNode>>,
    ) -> QueryTreeNode {
        debug!(
            "Building root query tree node: {}",
            graph.pretty_print_node(root_node_index)
        );

        let mut root_tree_node = Self::new_root(root_node_index);

        if edges.is_empty() {
            debug!("Path has no edges beyond the root.");
        } else {
            let first_subsequent_node =
                QueryTreeNode::from_path_segment_sequences(graph, edges, requirements_tree, 0);

            if let Some(first_subsequent_node) = first_subsequent_node {
                root_tree_node.children.push(first_subsequent_node);
            }
        }

        root_tree_node
    }

    fn internal_pretty_print(
        &self,
        graph: &Graph,
        indent_level: usize,
    ) -> Result<String, std::fmt::Error> {
        let mut result = String::new();
        let indent = "  ".repeat(indent_level);

        let edge_index = self.edge_from_parent.unwrap();
        let edge = graph.edge(edge_index).unwrap();
        let move_str = format!("{}", edge);

        let node = graph.node(self.node_index).unwrap();
        let tail_str = format!("{}", node);

        if !self.requirements.is_empty() {
            write!(result, "\n{}🧩 #{:?} [", indent, edge_index)?;

            for req_step in self.requirements.iter() {
                write!(
                    result,
                    "{}",
                    req_step.internal_pretty_print(graph, indent_level)?
                )?;
            }

            write!(result, "\n{}]", indent)?;
        }

        match edge {
            Edge::EntityMove(_) => write!(
                result,
                "\n{}{} {} #{:?}",
                indent, move_str, tail_str, edge_index
            ),
            _ => write!(
                result,
                "\n{}{} of {} #{:?}",
                indent, move_str, tail_str, edge_index
            ),
        }?;

        for sub_step in self.children.iter() {
            write!(
                result,
                "{}",
                sub_step.internal_pretty_print(graph, indent_level + 1)?
            )?;
        }

        Ok(result)
    }

    pub fn pretty_print(
        &self,
        graph: &Graph,
        indent_level: usize,
    ) -> Result<String, std::fmt::Error> {
        let mut result = String::new();

        match self.edge_from_parent {
            Some(_) => write!(
                result,
                "{}",
                self.internal_pretty_print(graph, indent_level)?
            )?,
            None => {
                for sub_step in self.children.iter() {
                    write!(
                        result,
                        "{}",
                        sub_step.internal_pretty_print(graph, indent_level + 1)?
                    )?;
                }
            }
        };

        Ok(result)
    }
}
