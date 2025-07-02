use petgraph::graph::{EdgeIndex, NodeIndex};
use std::fmt::Write;
use std::sync::Arc;
use tracing::{instrument, trace};

use crate::{
    ast::{arguments::ArgumentsMap, merge_path::Condition},
    graph::{edge::Edge, error::GraphError, Graph},
    planner::walker::path::{OperationPath, PathSegment, SelectionAttributes},
};

use super::query_tree::QueryTree;

/// Represents the position of a mutation root field,
/// in the operation's selection set.
pub type MutationFieldPosition = Option<usize>;

#[derive(Debug, Clone)]
pub struct QueryTreeNode {
    /// The underlying graph node this query tree node corresponds to
    pub node_index: NodeIndex,
    /// The edge from the parent QueryTreeNode that led to this node (null for root)
    pub edge_from_parent: Option<EdgeIndex>,
    /// Nodes required to execute the move
    pub requirements: Vec<Arc<QueryTreeNode>>,
    pub children: Vec<Arc<QueryTreeNode>>,
    pub selection_attributes: Option<SelectionAttributes>,
    pub condition: Option<Condition>,
    /// Distinguishes nodes originating from different top-level mutation fields.
    pub mutation_field_position: MutationFieldPosition,
}

/// Implements the `PartialEq` trait for `QueryTreeNode` to allow comparison based on node index, edge from parent, and selection attributes.
/// This is also the way to "fingerprint" a node in the query tree, as it allows us to determine if two nodes are effectively
/// the same in terms of their position and attributes in the graph while merging.
impl PartialEq for QueryTreeNode {
    fn eq(&self, other: &Self) -> bool {
        self.node_index == other.node_index
            && self.edge_from_parent == other.edge_from_parent
            && self.selection_attributes == other.selection_attributes
            && self.mutation_field_position == other.mutation_field_position
            && self.condition == other.condition
    }
}

fn merge_query_tree_node_list(
    target_list: &mut Vec<Arc<QueryTreeNode>>,
    source_list: &[Arc<QueryTreeNode>],
) {
    if source_list.is_empty() {
        return; // nothing to merge from the source
    }

    for source_node in source_list.iter() {
        let matching_target_node = target_list
            .iter_mut()
            .find(|target_node| target_node.as_ref() == source_node.as_ref());

        match matching_target_node {
            Some(target_node) => {
                let target_node_mut = Arc::make_mut(target_node);
                target_node_mut.merge_nodes(source_node.as_ref());
            }
            None => {
                // No match found, add a clone of the source Arc to the target list.
                target_list.push(Arc::clone(source_node));
            }
        }
    }
}

impl QueryTreeNode {
    pub fn new(
        node_index: &NodeIndex,
        edge_from_parent: Option<&EdgeIndex>,
        selection_attributes: Option<&SelectionAttributes>,
        condition: Option<&Condition>,
    ) -> Self {
        QueryTreeNode {
            node_index: *node_index,
            edge_from_parent: edge_from_parent.cloned(),
            requirements: Vec::new(),
            children: Vec::new(),
            selection_attributes: selection_attributes.cloned(),
            mutation_field_position: None,
            // TODO: improve it
            condition: condition.cloned(),
        }
    }

    pub fn selection_arguments(&self) -> Option<&ArgumentsMap> {
        self.selection_attributes
            .as_ref()
            .and_then(|v| v.arguments.as_ref())
    }

    pub fn selection_alias(&self) -> Option<&str> {
        self.selection_attributes
            .as_ref()
            .and_then(|v| v.alias.as_deref())
    }

    pub fn new_root(node_index: &NodeIndex) -> Self {
        QueryTreeNode::new(node_index, None, None, None)
    }

    pub fn merge_nodes(&mut self, other: &Self) {
        merge_query_tree_node_list(&mut self.children, &other.children);
        merge_query_tree_node_list(&mut self.requirements, &other.requirements);
    }

    pub fn from_paths(
        graph: &Graph,
        paths: &[OperationPath],
        mutation_field_position: MutationFieldPosition,
    ) -> Result<Option<Arc<Self>>, GraphError> {
        if paths.is_empty() {
            return Ok(None);
        }

        let mut trees = paths
            .iter()
            .map(|path| {
                // TODO: cover with ?
                QueryTree::from_path(graph, path, mutation_field_position)
                    .expect("expected tree to be built but it failed")
            })
            .collect::<Vec<_>>();

        if trees.len() == 1 {
            return Ok(Some(trees.remove(0).root));
        }

        Ok(Some(QueryTree::merge_trees(trees).root))
    }

    #[instrument(level = "trace",skip(graph, segments), fields(
      total_segments = segments.len()
    ))]
    fn from_path_segment_sequences(
        graph: &Graph,
        segments: &[Arc<PathSegment>],
        current_index: usize,
        mutation_field_position: MutationFieldPosition,
    ) -> Result<Option<Arc<Self>>, GraphError> {
        if current_index >= segments.len() {
            return Ok(None);
        }

        let segment_at_index: &Arc<PathSegment> = &segments[current_index];
        let edge_at_index = &segment_at_index.edge_index;
        let requirements_tree_at_index = &segment_at_index.requirement_tree;
        let selection_attributes_at_index = &segment_at_index.selection_attributes;

        trace!(
            "Processing edge: {}",
            graph.pretty_print_edge(*edge_at_index, false)
        );

        trace!("Condition: {:?}", segment_at_index.condition);

        // Creates the QueryTreeNode representing the state after traversing this edge
        let tail_node_index = graph.get_edge_tail(edge_at_index)?;
        let mut tree_node = QueryTreeNode::new(
            &tail_node_index,
            Some(edge_at_index),
            (*selection_attributes_at_index).as_ref(),
            segment_at_index.condition.as_ref(),
        );

        // Only apply the position to the first node created from the segments
        if current_index == 0 {
            tree_node.mutation_field_position = mutation_field_position;
        }

        if let Some(requirements_tree_arc) = requirements_tree_at_index {
            tree_node
                .requirements
                .push(Arc::clone(requirements_tree_arc));
        }

        let subsequent_query_tree_node =
            Self::from_path_segment_sequences(graph, segments, current_index + 1, None)?;

        match subsequent_query_tree_node {
            Some(subsequent_query_tree_node) => {
                trace!("Adding subsequent step as child");
                tree_node.children.push(subsequent_query_tree_node);
            }
            None => {
                trace!("No subsequent steps (leaf or end of path)");
            }
        }
        Ok(Some(Arc::new(tree_node)))
    }

    #[instrument(level = "trace",skip_all, fields(
      root_node = graph.pretty_print_node(root_node_index),
      segments_count = segments.len()
    ))]
    pub fn create_root_for_path_sequences(
        graph: &Graph,
        root_node_index: &NodeIndex,
        segments: &Vec<Arc<PathSegment>>,
        mutation_field_position: MutationFieldPosition,
    ) -> Result<QueryTreeNode, GraphError> {
        trace!(
            "Building root query tree node: {}",
            graph.pretty_print_node(root_node_index)
        );

        let mut root_tree_node = Self::new_root(root_node_index);

        if segments.is_empty() {
            trace!("Path has no edges beyond the root.");
        } else {
            let subsequent_node = QueryTreeNode::from_path_segment_sequences(
                graph,
                segments.as_slice(),
                0,
                mutation_field_position,
            )?;

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

        let edge_index = self
            .edge_from_parent
            .expect("internal_pretty_print called on a node without a parent edge");
        let edge = graph.edge(edge_index).unwrap();

        let node = graph.node(self.node_index).unwrap();
        let tail_str = format!("{}", node);

        let condition_str = match self.condition.as_ref() {
            Some(condition) => format!(" {}", condition),
            None => "".to_string(),
        };

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
            _ => {
                let args_str = self
                    .selection_arguments()
                    .map(|v| match v.is_empty() {
                        true => "".to_string(),
                        false => format!("({})", v),
                    })
                    .unwrap_or("".to_string());
                write!(
                    result,
                    "\n{}{}{} of {}{}",
                    indent,
                    edge.display_name(),
                    args_str,
                    tail_str,
                    condition_str
                )
            }
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
