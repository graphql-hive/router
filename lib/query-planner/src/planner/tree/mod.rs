use petgraph::graph::{EdgeIndex, NodeIndex};
use tracing::{debug, instrument};

use crate::graph::Graph;

use super::walker::path::OperationPath;

#[derive(Debug, Clone)]
pub struct QueryTreeNode {
    /// The underlying graph node this query tree node corresponds to
    pub node_index: NodeIndex,
    /// The edge from the parent QueryTreeNode that led to this node (null for root)
    pub edge_from_parent: Option<EdgeIndex>,
    /// Nodes required to execute the move
    requirements: Vec<QueryTreeNode>,
    children: Vec<QueryTreeNode>,
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
    fn create_root_for_path_sequences(
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
}

#[derive(Debug, Clone)]
pub struct QueryTree {
    root: QueryTreeNode,
}

impl QueryTree {
    fn new(root: QueryTreeNode) -> Self {
        QueryTree { root }
    }

    #[instrument(skip(graph))]
    fn from_path(graph: &Graph, path: &OperationPath) -> Self {
        debug!(
            "building tree directly from path starting at: {}",
            graph.pretty_print_node(&path.root_node)
        );

        let edges = path.get_edges();
        let requirements_tree = path.get_requirement_tree();

        let root_node = QueryTreeNode::create_root_for_path_sequences(
            graph,
            &path.root_node,
            &edges,
            &requirements_tree,
        );

        QueryTree::new(root_node)
    }

    #[instrument]
    pub fn merge_trees(mut trees: Vec<QueryTree>) -> QueryTree {
        let mut accumulator = trees.remove(0);

        for item in trees {
            accumulator.root = accumulator.root.merge_nodes(item.root);
        }

        accumulator
    }
}
