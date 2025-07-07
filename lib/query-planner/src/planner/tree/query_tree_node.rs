use bumpalo::collections::Vec as BumpVec;
use petgraph::graph::{EdgeIndex, NodeIndex};
use std::fmt::Write;
use tracing::{instrument, trace};

use crate::{
    ast::arguments::ArgumentsMap,
    graph::{edge::Edge, error::GraphError, Graph},
    planner::walker::{
        path::{OperationPath, PathSegment, SelectionAttributes},
        WalkContext,
    },
};

use super::query_tree::QueryTree;

/// Represents the position of a mutation root field,
/// in the operation's selection set.
pub type MutationFieldPosition = Option<usize>;

#[derive(Debug)]
pub struct QueryTreeNode<'bump> {
    /// The underlying graph node this query tree node corresponds to
    pub node_index: NodeIndex,
    /// The edge from the parent QueryTreeNode that led to this node (null for root)
    pub edge_from_parent: Option<EdgeIndex>,
    /// Nodes required to execute the move
    pub requirements: BumpVec<'bump, &'bump Self>,
    pub children: BumpVec<'bump, &'bump Self>,
    pub selection_attributes: Option<SelectionAttributes>,
    /// Distinguishes nodes originating from different top-level mutation fields.
    pub mutation_field_position: MutationFieldPosition,
}

/// Implements the `PartialEq` trait for `QueryTreeNode` to allow comparison based on node index, edge from parent, and selection attributes.
/// This is also the way to "fingerprint" a node in the query tree, as it allows us to determine if two nodes are effectively
/// the same in terms of their position and attributes in the graph while merging.
impl<'bump> PartialEq for QueryTreeNode<'bump> {
    fn eq(&self, other: &Self) -> bool {
        self.node_index == other.node_index
            && self.edge_from_parent == other.edge_from_parent
            && self.selection_attributes == other.selection_attributes
            && self.mutation_field_position == other.mutation_field_position
    }
}

fn merge_query_tree_node_list<'bump>(
    ctx: &WalkContext<'bump>,
    target_list: &mut BumpVec<'bump, &'bump QueryTreeNode<'bump>>,
    source_list: &BumpVec<'bump, &'bump QueryTreeNode<'bump>>,
) {
    if source_list.is_empty() {
        return; // nothing to merge from the source
    }

    'source_loop: for source_node in source_list.iter() {
        for i in 0..target_list.len() {
            // We need to use index-based access to be able to replace the element
            let target_node = target_list[i];

            if target_node == *source_node {
                // Found a matching node. We need to merge them.
                // Since nodes in the arena are immutable by reference, we clone the target,
                // merge the source into the clone, and replace the original in the list.
                let mut new_node = target_node.clone_in(ctx);
                new_node.merge_nodes(ctx, source_node);
                // Replace the old node with the new merged one.
                target_list[i] = ctx.arena.alloc(new_node);
                continue 'source_loop;
            }
        }
        // No match found, add a clone of the source node to the target list.
        target_list.push(ctx.arena.alloc(source_node.clone_in(ctx)));
    }
}

impl<'bump> QueryTreeNode<'bump> {
    pub fn new(
        ctx: &WalkContext<'bump>,
        node_index: &NodeIndex,
        edge_from_parent: Option<&EdgeIndex>,
        selection_attributes: Option<&SelectionAttributes>,
    ) -> Self {
        QueryTreeNode {
            node_index: *node_index,
            edge_from_parent: edge_from_parent.cloned(),
            requirements: BumpVec::new_in(ctx.arena),
            children: BumpVec::new_in(ctx.arena),
            selection_attributes: selection_attributes.cloned(),
            mutation_field_position: None,
        }
    }

    // Custom clone method to handle bump allocation by creating a deep copy in the arena.
    pub fn clone_in(&self, ctx: &WalkContext<'bump>) -> Self {
        let mut new_node = Self::new(
            ctx,
            &self.node_index,
            self.edge_from_parent.as_ref(),
            self.selection_attributes.as_ref(),
        );
        new_node.mutation_field_position = self.mutation_field_position;

        new_node
            .children
            .extend(self.children.iter().map(|child| child.clone_in_ctx(ctx)));
        new_node
            .requirements
            .extend(self.requirements.iter().map(|req| req.clone_in_ctx(ctx)));

        new_node
    }

    // Helper to clone self into a bump-allocated reference.
    fn clone_in_ctx(&self, ctx: &WalkContext<'bump>) -> &'bump Self {
        ctx.arena.alloc(self.clone_in(ctx))
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

    pub fn new_root(ctx: &WalkContext<'bump>, node_index: &NodeIndex) -> Self {
        Self::new(ctx, node_index, None, None)
    }

    pub fn merge_nodes(&mut self, ctx: &WalkContext<'bump>, other: &Self) {
        merge_query_tree_node_list(ctx, &mut self.children, &other.children);
        merge_query_tree_node_list(ctx, &mut self.requirements, &other.requirements);
    }

    pub fn from_paths(
        ctx: &WalkContext<'bump>,
        paths: &BumpVec<'bump, OperationPath<'bump>>,
        mutation_field_position: MutationFieldPosition,
    ) -> Result<Option<&'bump Self>, GraphError> {
        if paths.is_empty() {
            return Ok(None);
        }

        let mut trees = BumpVec::new_in(ctx.arena);
        for path in paths.iter() {
            trees.push(QueryTree::from_path(ctx, path, mutation_field_position)?);
        }

        if trees.len() == 1 {
            // .remove(0) is not available on BumpVec, so we do this instead.
            return Ok(Some(trees.into_iter().next().unwrap().root));
        }

        Ok(Some(QueryTree::merge_trees(ctx, trees).root))
    }

    #[instrument(level = "trace",skip(ctx, segments), fields(
      total_segments = segments.len()
    ))]
    fn from_path_segment_sequences(
        ctx: &WalkContext<'bump>,
        segments: &[&'bump PathSegment<'bump>],
        current_index: usize,
        mutation_field_position: MutationFieldPosition,
    ) -> Result<Option<&'bump Self>, GraphError> {
        if current_index >= segments.len() {
            return Ok(None);
        }

        let segment_at_index = segments[current_index];
        let edge_at_index = &segment_at_index.edge_index;
        let requirements_tree_at_index = &segment_at_index.requirement_tree;
        let selection_attributes_at_index = &segment_at_index.selection_attributes;

        trace!(
            "Processing edge: {}",
            ctx.graph.pretty_print_edge(*edge_at_index, false)
        );

        let tail_node_index = ctx.graph.get_edge_tail(edge_at_index)?;
        let mut tree_node = QueryTreeNode::new(
            ctx,
            &tail_node_index,
            Some(edge_at_index),
            selection_attributes_at_index.as_ref(),
        );

        if current_index == 0 {
            tree_node.mutation_field_position = mutation_field_position;
        }

        if let Some(requirements_tree_node) = requirements_tree_at_index {
            tree_node.requirements.push(requirements_tree_node);
        }

        let subsequent_query_tree_node =
            Self::from_path_segment_sequences(ctx, segments, current_index + 1, None)?;

        if let Some(subsequent_node) = subsequent_query_tree_node {
            trace!("Adding subsequent step as child");
            tree_node.children.push(subsequent_node);
        } else {
            trace!("No subsequent steps (leaf or end of path)");
        }

        Ok(Some(ctx.arena.alloc(tree_node)))
    }

    #[instrument(level = "trace",skip_all, fields(
      root_node = ctx.graph.pretty_print_node(root_node_index),
      segments_count = segments.len()
    ))]
    pub fn create_root_for_path_sequences(
        ctx: &WalkContext<'bump>,
        root_node_index: &NodeIndex,
        segments: &BumpVec<'bump, &'bump PathSegment<'bump>>,
        mutation_field_position: MutationFieldPosition,
    ) -> Result<&'bump Self, GraphError> {
        trace!(
            "Building root query tree node: {}",
            ctx.graph.pretty_print_node(root_node_index)
        );

        let mut root_tree_node = Self::new_root(ctx, root_node_index);

        if segments.is_empty() {
            trace!("Path has no edges beyond the root.");
        } else {
            let subsequent_node = QueryTreeNode::from_path_segment_sequences(
                ctx,
                segments.as_slice(),
                0,
                mutation_field_position,
            )?;

            if let Some(subsequent_node) = subsequent_node {
                root_tree_node.children.push(subsequent_node);
            }
        }

        Ok(ctx.arena.alloc(root_tree_node))
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
                    "\n{}{}{} of {}",
                    indent,
                    edge.display_name(),
                    args_str,
                    tail_str
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
