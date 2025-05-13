use petgraph::graph::{EdgeIndex, NodeIndex};
use std::fmt::Write;
use tracing::{debug, instrument};

use crate::{
    graph::{edge::Edge, error::GraphError, Graph},
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

impl PartialEq for QueryTreeNode {
    fn eq(&self, other: &Self) -> bool {
        self.node_index == other.node_index && self.edge_from_parent == other.edge_from_parent
    }
}

fn merge_query_tree_node_list(target_list: &mut Vec<QueryTreeNode>, source_list: &[QueryTreeNode]) {
    if source_list.is_empty() {
        return; // nothing to merge from the source
    }

    for source_node in source_list.iter() {
        let matching_target_node = target_list
            .iter_mut()
            .find(|target_node| **target_node == *source_node);

        match matching_target_node {
            Some(target_node) => {
                // Match found, recursively merge the content
                target_node.merge_nodes(source_node);
            }
            None => {
                // No match found, add the source node (and its subtree) to the target list
                target_list.push(source_node.clone());
            }
        }
    }
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

    pub fn merge_nodes(&mut self, other: &Self) -> Self {
        merge_query_tree_node_list(&mut self.children, &other.children);
        merge_query_tree_node_list(&mut self.requirements, &other.requirements);

        QueryTreeNode {
            node_index: self.node_index,
            edge_from_parent: self.edge_from_parent,
            requirements: self.requirements.clone(),
            children: self.children.clone(),
        }
    }

    // #[instrument(skip(graph), fields(
    //   paths = paths.iter().map(|path| path.pretty_print(graph)).collect::<Vec<String>>().join(", ")
    // ))]
    pub fn from_paths(graph: &Graph, paths: &[OperationPath]) -> Result<Option<Self>, GraphError> {
        if paths.is_empty() {
            return Ok(None);
        }

        let mut trees = paths
            .iter()
            .map(|path| {
                QueryTree::from_path(graph, path).expect("expected tree to be built but it failed")
            })
            .collect::<Vec<_>>();

        if trees.len() == 1 {
            return Ok(Some(trees.remove(0).root));
        }

        Ok(Some(QueryTree::merge_trees(trees).root))
    }

    #[instrument(skip(graph, requirements_trees), fields(
      total_edges = edges.len()
    ))]
    fn from_path_segment_sequences(
        graph: &Graph,
        edges: &[EdgeIndex],
        requirements_trees: &[Option<&QueryTreeNode>],
        current_index: usize,
    ) -> Result<Option<Self>, GraphError> {
        if current_index >= edges.len() {
            return Ok(None);
        }

        let edge_at_index = edges[current_index];
        let requirements_tree = requirements_trees[current_index];

        debug!(
            "Processing edge: {}",
            graph.pretty_print_edge(edge_at_index, false)
        );

        // Creates the QueryTreeNode representing the state after traversing this edge
        let tail_node_index = graph.get_edge_tail(&edge_at_index)?;
        let mut tree_node = QueryTreeNode::new(&tail_node_index, Some(&edge_at_index));

        if let Some(requirements_tree) = requirements_tree {
            tree_node.requirements.push(requirements_tree.clone());
        }

        let subsequent_query_tree_node =
            Self::from_path_segment_sequences(graph, edges, requirements_trees, current_index + 1)?;

        match subsequent_query_tree_node {
            Some(subsequent_query_tree_node) => {
                debug!("Adding subsequent step as child");
                tree_node.children.push(subsequent_query_tree_node);
            }
            None => {
                debug!("No subsequent steps (leaf or end of path)");
            }
        }

        Ok(Some(tree_node))
    }

    #[instrument(skip(graph))]
    pub fn create_root_for_path_sequences(
        graph: &Graph,
        root_node_index: &NodeIndex,
        edges: &Vec<EdgeIndex>,
        requirements_tree: &Vec<Option<&QueryTreeNode>>,
    ) -> Result<QueryTreeNode, GraphError> {
        debug!(
            "Building root query tree node: {}",
            graph.pretty_print_node(root_node_index)
        );

        let mut root_tree_node = Self::new_root(root_node_index);

        if edges.is_empty() {
            debug!("Path has no edges beyond the root.");
        } else {
            let subsequent_node =
                QueryTreeNode::from_path_segment_sequences(graph, edges, requirements_tree, 0)?;

            if let Some(subsequent_node) = subsequent_node {
                root_tree_node.children.push(subsequent_node);
            }
        }

        Ok(root_tree_node)
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

        let node = graph.node(self.node_index).unwrap();
        let tail_str = format!("{}", node);

        if !self.requirements.is_empty() {
            write!(result, "\n{}🧩 [", indent)?;

            for req_step in self.requirements.iter() {
                write!(result, "{}", req_step.pretty_print(graph, indent_level)?)?;
            }

            write!(result, "\n{}]", indent)?;
        }

        match edge {
            Edge::EntityMove(_) => {
                write!(result, "\n{}🔑 {}", indent, tail_str)
            }
            Edge::SubgraphEntrypoint { .. } => {
                write!(result, "\n{}🚪 ({})", indent, tail_str)
            }
            _ => write!(
                result,
                "\n{}{} of {}",
                indent,
                edge.display_name(),
                tail_str
            ),
        }?;

        for sub_step in self.children.iter() {
            write!(
                result,
                "{}",
                sub_step.pretty_print(graph, indent_level + 1)?
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
                        sub_step.pretty_print(graph, indent_level + 1)?
                    )?;
                }
            }
        };

        Ok(result)
    }
}
