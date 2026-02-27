use std::collections::{HashMap, VecDeque};

use petgraph::{graph::NodeIndex, visit::EdgeRef};

use crate::{
    planner::plan_nodes::ConditionNode, state::supergraph_state::SupergraphState,
    utils::cancellation::CancellationToken,
};

use super::{
    error::QueryPlanError,
    fetch::fetch_graph::FetchGraph,
    plan_nodes::{
        FetchNode, ParallelNode, PlanNode, QueryPlan, RepresentationReusePlan, SequenceNode,
    },
};

/// Tracks the in-degree of FetchGraph (DAG) in a dependency graph.
/// The in-degree of a step is the number of its prerequisite parent steps
/// that have not yet been processed. A step is "fulfilled" (ready to be processed)
/// when its in-degree becomes zero.
pub struct InDegree<'a> {
    state: HashMap<NodeIndex, usize>,
    fetch_graph: &'a FetchGraph,
}

impl<'a> InDegree<'a> {
    pub fn new(fetch_graph: &'a FetchGraph) -> Result<Self, QueryPlanError> {
        let mut state: HashMap<NodeIndex, usize> = HashMap::new();
        let root_index = fetch_graph.root_index.ok_or(QueryPlanError::NoRoot)?;

        // For all steps, the initial in-degree is set to the total number of their parents
        fetch_graph.bfs(root_index, |step_index, _| {
            state.insert(
                *step_index,
                fetch_graph
                    .parents_of(*step_index)
                    // skip roots
                    .filter(|edge| edge.source() != root_index)
                    .count(),
            );
            false // never stop traversing
        });

        Ok(Self { state, fetch_graph })
    }

    /// Marks a `FetchStep` as processed. This involves decrementing the in-degree
    /// of all its direct children as one of their parent dependencies
    /// has been met.
    pub fn mark_as_processed(&mut self, index: NodeIndex) {
        for edge in self.fetch_graph.children_of(index) {
            let child_index = edge.target();
            let current = self.state.get(&child_index);

            if let Some(in_degree) = current {
                if *in_degree == 0 {
                    panic!("In-degree was 0");
                }

                self.state.insert(child_index, in_degree - 1);
            } else {
                panic!("Attempt to decrease an in-degree of a non-existing step");
            }
        }
    }

    /// Checks if a `FetchStep` is "fulfilled," meaning all its parent dependencies have been met.
    pub fn is_fulfilled(&self, child_index: NodeIndex) -> bool {
        self.state
            .get(&child_index)
            .expect("In-degree record missing")
            == &0
    }
}

pub static QUERY_PLAN_KIND: &str = "QueryPlan";

const REPRESENTATION_REUSE_PLAN_VERSION: u8 = 2;

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct RepresentationFetchSignature {
    service_name: String,
    operation_hash: u64,
    operation_name: Option<String>,
    variable_usages: Option<Vec<String>>,
}

fn build_representation_reuse_plan(node: &PlanNode) -> Option<RepresentationReusePlan> {
    let mut signatures: HashMap<RepresentationFetchSignature, Vec<i64>> = HashMap::new();
    collect_representation_fetch_signatures(node, &mut signatures);

    let mut groups = signatures
        .into_values()
        .filter_map(|mut fetch_ids| {
            if fetch_ids.len() < 2 {
                return None;
            }

            fetch_ids.sort_unstable();
            fetch_ids.dedup();

            if fetch_ids.len() < 2 {
                return None;
            }

            Some(fetch_ids)
        })
        .collect::<Vec<_>>();

    groups.sort_unstable_by(|a, b| a.first().cmp(&b.first()).then_with(|| a.cmp(b)));

    if groups.is_empty() {
        None
    } else {
        let mut fetch_id_to_group_id = HashMap::new();
        for (group_id, group) in groups.iter().enumerate() {
            for &fetch_id in group {
                fetch_id_to_group_id.insert(fetch_id, group_id);
            }
        }

        Some(RepresentationReusePlan {
            version: REPRESENTATION_REUSE_PLAN_VERSION,
            groups,
            fetch_id_to_group_id,
        })
    }
}

fn collect_representation_fetch_signatures(
    node: &PlanNode,
    signatures: &mut HashMap<RepresentationFetchSignature, Vec<i64>>,
) {
    match node {
        PlanNode::Fetch(_) => {}
        PlanNode::Flatten(flatten_node) => {
            if let PlanNode::Fetch(fetch_node) = flatten_node.node.as_ref() {
                if fetch_node.requires.is_some() {
                    let signature = representation_fetch_signature(fetch_node);
                    signatures.entry(signature).or_default().push(fetch_node.id);
                }
            }

            collect_representation_fetch_signatures(flatten_node.node.as_ref(), signatures);
        }
        PlanNode::Sequence(sequence_node) => {
            for child in &sequence_node.nodes {
                collect_representation_fetch_signatures(child, signatures);
            }
        }
        PlanNode::Parallel(parallel_node) => {
            for child in &parallel_node.nodes {
                collect_representation_fetch_signatures(child, signatures);
            }
        }
        PlanNode::Condition(condition_node) => {
            if let Some(if_clause) = condition_node.if_clause.as_ref() {
                collect_representation_fetch_signatures(if_clause.as_ref(), signatures);
            }

            if let Some(else_clause) = condition_node.else_clause.as_ref() {
                collect_representation_fetch_signatures(else_clause.as_ref(), signatures);
            }
        }
        PlanNode::Subscription(subscription_node) => {
            collect_representation_fetch_signatures(subscription_node.primary.as_ref(), signatures);
        }
        PlanNode::Defer(defer_node) => {
            if let Some(node) = defer_node.primary.node.as_ref() {
                collect_representation_fetch_signatures(node.as_ref(), signatures);
            }

            for deferred in &defer_node.deferred {
                if let Some(node) = deferred.node.as_ref() {
                    collect_representation_fetch_signatures(node.as_ref(), signatures);
                }
            }
        }
    }
}

fn representation_fetch_signature(fetch_node: &FetchNode) -> RepresentationFetchSignature {
    RepresentationFetchSignature {
        service_name: fetch_node.service_name.clone(),
        operation_hash: fetch_node.operation.hash,
        operation_name: fetch_node.operation_name.clone(),
        variable_usages: fetch_node
            .variable_usages
            .as_ref()
            .map(|variable_usages| variable_usages.iter().cloned().collect()),
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
        if in_degrees.is_fulfilled(child_index) {
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
            in_degrees.mark_as_processed(step_index);

            for child_edge in fetch_graph.children_of(step_index) {
                cancellation_token.bail_if_cancelled()?;
                let child_index = child_edge.target();
                if child_index == root_index {
                    return Err(QueryPlanError::Internal(String::from(
                        "Visited child step is a root step. It should not happen",
                    )));
                }

                if in_degrees.is_fulfilled(child_index) {
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
                QueryPlanError::Internal(String::from("Was was expected to be of length 1")),
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

    let root_node = match overall_plan_sequence.len() == 1 {
        true => overall_plan_sequence.into_iter().next().unwrap(),
        false => PlanNode::Sequence(SequenceNode {
            nodes: overall_plan_sequence,
        }),
    };

    let representation_reuse_plan = build_representation_reuse_plan(&root_node);

    Ok(QueryPlan {
        kind: QUERY_PLAN_KIND,
        node: Some(root_node),
        representation_reuse_plan,
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

fn merge_two_condition_nodes(a: PlanNode, b: PlanNode) -> PlanNode {
    let (mut a, mut b) = match (a, b) {
        (PlanNode::Condition(c1), PlanNode::Condition(c2)) => (c1, c2),
        _ => panic!("Can only merge two ConditionNodes"),
    };

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
        match (acc.last_mut(), &current_node) {
            // Check if the last node and the current node
            // have compatible conditions
            (Some(PlanNode::Condition(last_cond)), PlanNode::Condition(current_cond))
                if are_conditions_compatible(last_cond, current_cond) =>
            {
                let last_node = acc.pop().unwrap();
                let merged_node = merge_two_condition_nodes(last_node, current_node);
                acc.push(merged_node);
            }
            _ => {
                acc.push(current_node);
            }
        }
        acc
    })
}
