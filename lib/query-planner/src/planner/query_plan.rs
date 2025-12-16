use std::collections::VecDeque;

use petgraph::{
    graph::NodeIndex,
    visit::{EdgeRef, NodeIndexable},
};

use crate::{
    planner::plan_nodes::ConditionNode, state::supergraph_state::SupergraphState,
    utils::cancellation::CancellationToken,
};

use super::{
    error::QueryPlanError,
    fetch::fetch_graph::FetchGraph,
    plan_nodes::{ParallelNode, PlanNode, QueryPlan, SequenceNode},
};

/// Tracks the in-degree of FetchGraph (DAG) in a dependency graph.
/// The in-degree of a step is the number of its prerequisite parent steps
/// that have not yet been processed. A step is "fulfilled" (ready to be processed)
/// when its in-degree becomes zero.
///
/// Internally, values are stored as `in_degree + 1` to allow using 0 as the
/// "unvisited" sentinel. This enables `vec![0; n]` initialization which uses
/// `calloc` for better memory performance (zero-pages from OS, lazy allocation).
pub struct InDegree<'a> {
    state: Box<[u32]>,
    fetch_graph: &'a FetchGraph,
}

impl<'a> InDegree<'a> {
    /// Sentinel value for unvisited nodes (stored as 0, meaning in_degree + 1 = 0)
    const UNVISITED: u32 = 0;

    pub fn new(fetch_graph: &'a FetchGraph) -> Result<Self, QueryPlanError> {
        let root_index = fetch_graph.root_index.ok_or(QueryPlanError::NoRoot)?;
        // Zero-init is optimized by the allocator (calloc), avoiding memset overhead
        let mut state: Vec<u32> = vec![0; fetch_graph.graph.node_bound()];
        let mut overflow_error: Option<QueryPlanError> = None;

        // For all steps, the initial in-degree is set to the total number of their parents
        fetch_graph.bfs(root_index, |step_index, _| {
            let in_degree_usize = fetch_graph
                .parents_of(*step_index)
                // skip roots
                .filter(|edge| edge.source() != root_index)
                .count();

            // Store as in_degree + 1 to reserve 0 as the "unvisited" sentinel
            let Some(stored_value) = u32::try_from(in_degree_usize)
                .ok()
                .and_then(|v| v.checked_add(1))
            else {
                overflow_error = Some(QueryPlanError::Internal(format!(
                    "In-degree overflow for step {}: {in_degree_usize}",
                    step_index.index(),
                )));
                return true; // stop traversal early
            };

            state[step_index.index()] = stored_value;
            false // never stop traversing
        });

        if let Some(err) = overflow_error {
            return Err(err);
        }

        Ok(Self {
            state: state.into_boxed_slice(),
            fetch_graph,
        })
    }

    /// Marks a `FetchStep` as processed. This involves decrementing the in-degree
    /// of all its direct children as one of their parent dependencies
    /// has been met.
    pub fn mark_as_processed(&mut self, index: NodeIndex) -> Result<(), QueryPlanError> {
        for edge in self.fetch_graph.children_of(index) {
            let child_index = edge.target();
            let entry = self.state.get_mut(child_index.index()).ok_or_else(|| {
                QueryPlanError::Internal(format!(
                    "In-degree vector missing entry for child step {}",
                    child_index.index()
                ))
            })?;

            if *entry == Self::UNVISITED {
                return Err(QueryPlanError::Internal(format!(
                    "Attempt to decrease in-degree of a non-existing step {}",
                    child_index.index()
                )));
            }

            // stored_value = in_degree + 1, so stored_value == 1 means in_degree == 0
            if *entry == 1 {
                return Err(QueryPlanError::Internal(format!(
                    "In-degree was 0 for step {}",
                    child_index.index()
                )));
            }

            *entry -= 1;
        }
        Ok(())
    }

    /// Checks if a `FetchStep` is "fulfilled," meaning all its parent dependencies have been met.
    pub fn is_fulfilled(&self, child_index: NodeIndex) -> Result<bool, QueryPlanError> {
        let v = *self.state.get(child_index.index()).ok_or_else(|| {
            QueryPlanError::Internal(format!(
                "In-degree vector missing entry for step {}",
                child_index.index()
            ))
        })?;

        if v == Self::UNVISITED {
            return Err(QueryPlanError::Internal(format!(
                "In-degree record missing for step {}",
                child_index.index()
            )));
        }

        // stored_value = in_degree + 1, so fulfilled when stored_value == 1
        Ok(v == 1)
    }
}

#[tracing::instrument(level = "trace", skip_all)]
pub fn build_query_plan_from_fetch_graph(
    fetch_graph: FetchGraph,
    supergraph: &SupergraphState,
    cancellation_token: &CancellationToken,
) -> Result<QueryPlan, QueryPlanError> {
    let root_index = fetch_graph.root_index.ok_or(QueryPlanError::NoRoot)?;

    let mut in_degrees = InDegree::new(&fetch_graph)?;
    let mut queue: VecDeque<NodeIndex> = VecDeque::new();
    let mut planned_nodes_count = 0;

    // Initialize the queue with direct children of the root_index.
    // Their in-degrees (calculated in InDegree::new) should be 0.
    for edge in fetch_graph.children_of(root_index) {
        let child_index = edge.target();
        if in_degrees.is_fulfilled(child_index)? {
            queue.push_back(child_index);
        } else {
            return Err(QueryPlanError::Internal(format!(
                "Root's child ({}) has more than one parent",
                child_index.index()
            )));
        }
    }

    let mut overall_plan_sequence: Vec<PlanNode> = Vec::new();

    while !queue.is_empty() {
        let mut current_wave_nodes: Vec<PlanNode> = Vec::new();
        let wave_size = queue.len();

        for _ in 0..wave_size {
            let step_index = queue
                .pop_front()
                .ok_or(QueryPlanError::Internal(String::from(
                    "Failed to pop a step from the queue. Queue should not be empty",
                )))?;

            let step_data = fetch_graph.get_step_data(step_index)?;
            current_wave_nodes.push(PlanNode::from_fetch_step(step_data, supergraph));
            planned_nodes_count += 1;
            in_degrees.mark_as_processed(step_index)?;

            for child_edge in fetch_graph.children_of(step_index) {
                cancellation_token.bail_if_cancelled()?;
                let child_index = child_edge.target();
                if child_index == root_index {
                    return Err(QueryPlanError::Internal(String::from(
                        "Visited child step is a root step. It should not happen",
                    )));
                }

                if in_degrees.is_fulfilled(child_index)? {
                    queue.push_back(child_index);
                }
            }
        }

        if current_wave_nodes.is_empty() {
            return Err(QueryPlanError::Internal(String::from(
                "Wave was empty. It should not happen as the queue was non-empty.",
            )));
        } else if current_wave_nodes.len() == 1 {
            overall_plan_sequence.push(current_wave_nodes.into_iter().next().ok_or(
                QueryPlanError::Internal(String::from("Wave was expected to be of length 1")),
            )?);
        } else {
            overall_plan_sequence.push(PlanNode::Parallel(ParallelNode {
                nodes: current_wave_nodes,
            }));
        }
    }

    let total_fetch_nodes = fetch_graph
        .step_indices()
        .filter(|&idx| idx != root_index)
        .count();

    if planned_nodes_count != total_fetch_nodes {
        return Err(QueryPlanError::Internal("Cycle detected".to_string()));
    }

    if overall_plan_sequence.is_empty() {
        if total_fetch_nodes == 0 {
            return Err(QueryPlanError::EmptyPlan);
        } else {
            return Err(QueryPlanError::Internal(
                "Plan is empty, but graph reported task nodes that were not planned.".to_string(),
            ));
        }
    }

    let overall_plan_sequence = optimize_plan_sequence(overall_plan_sequence);

    let root_node = match <[_; 1]>::try_from(overall_plan_sequence) {
        Ok([single]) => single,
        Err(nodes) => PlanNode::Sequence(SequenceNode { nodes }),
    };

    Ok(QueryPlan {
        kind: "QueryPlan".to_string(),
        node: Some(root_node),
    })
}

fn are_conditions_compatible(c1: &ConditionNode, c2: &ConditionNode) -> bool {
    // They refer to different variables
    if c1.condition != c2.condition {
        return false;
    }

    // Skip and Skip
    if c1.if_clause.is_none() && c2.if_clause.is_none() {
        return true;
    }

    // Include and Include
    if c1.else_clause.is_none() && c2.else_clause.is_none() {
        return true;
    }

    // Skip/Include and Include/Skip
    false
}

fn merge_two_condition_nodes(mut a: ConditionNode, mut b: ConditionNode) -> PlanNode {

    let is_if = a.if_clause.is_some();

    let mut inner_nodes: Vec<PlanNode> = a
        .if_clause
        .take()
        .or_else(|| a.else_clause.take())
        .map(|n| n.into_nodes())
        .unwrap_or_default()
        .into_iter()
        .chain(
            b.if_clause
                .take()
                .or_else(|| b.else_clause.take())
                .map(|n| n.into_nodes())
                .unwrap_or_default(),
        )
        .collect();

    // Use Sequence only if there are multiple nodes
    let merged_body = if inner_nodes.len() == 1 {
        inner_nodes.remove(0)
    } else {
        PlanNode::Sequence(SequenceNode { nodes: inner_nodes })
    };

    // Re-create the parent ConditionNode with the newly merged body.
    if is_if {
        PlanNode::Condition(ConditionNode {
            condition: a.condition,
            if_clause: Some(Box::new(merged_body)),
            else_clause: None,
        })
    } else {
        PlanNode::Condition(ConditionNode {
            condition: a.condition,
            if_clause: None,
            else_clause: Some(Box::new(merged_body)),
        })
    }
}

fn optimize_plan_sequence(nodes: Vec<PlanNode>) -> Vec<PlanNode> {
    nodes.into_iter().fold(Vec::new(), |mut acc, current_node| {
        match (&acc[..], &current_node) {
            // Check if the last node and the current node have compatible conditions
            ([.., PlanNode::Condition(last_ref)], PlanNode::Condition(current_ref))
                if are_conditions_compatible(last_ref, current_ref) =>
            {
                // Pop the last element - we know it exists and is a Condition from the pattern
                let Some(PlanNode::Condition(last_owned)) = acc.pop() else {
                    unreachable!(
                        "The slice pattern guarantees the last element is a ConditionNode."
                    );
                };
                let PlanNode::Condition(current_owned) = current_node else {
                    unreachable!(
                        "The match pattern guarantees the current node is a ConditionNode."
                    );
                };
                acc.push(merge_two_condition_nodes(last_owned, current_owned));
            }
            _ => {
                acc.push(current_node);
            }
        }
        acc
    })
}
