use std::collections::{HashMap, VecDeque};

use petgraph::{graph::NodeIndex, visit::EdgeRef};

use super::{
    error::QueryPlanError,
    fetch::fetch_graph::FetchGraph,
    plan_nodes::{ParallelNode, PlanNode, QueryPlan, SequenceNode},
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

#[tracing::instrument(skip_all)]
pub fn build_query_plan_from_fetch_graph(
    fetch_graph: FetchGraph,
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
            current_wave_nodes.push(step_data.into());
            planned_nodes_count += 1;
            in_degrees.mark_as_processed(step_index);

            for child_edge in fetch_graph.children_of(step_index) {
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

    let root_node = match overall_plan_sequence.len() == 1 {
        true => overall_plan_sequence.into_iter().next().unwrap(),
        false => PlanNode::Sequence(SequenceNode {
            nodes: overall_plan_sequence,
        }),
    };

    Ok(QueryPlan {
        kind: "QueryPlan".to_string(),
        node: Some(root_node),
    })
}
