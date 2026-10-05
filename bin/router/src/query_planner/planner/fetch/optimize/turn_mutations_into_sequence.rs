use std::collections::{BTreeMap, HashSet};

use petgraph::{
    graph::NodeIndex,
    visit::{Bfs, EdgeRef},
};
use tracing::instrument;

use crate::query_planner::{
    planner::fetch::{error::FetchGraphError, fetch_graph::FetchGraph, state::MultiTypeFetchStep},
    state::supergraph_state::{OperationKind, SubgraphName},
};

impl FetchGraph<MultiTypeFetchStep> {
    /// Root mutation fields run one after another. Consecutive fields of one
    /// subgraph form a group that goes in one request, and the subgraph runs them in order.
    /// A group is done only when every fetch it needs is done, entity calls included, so the
    /// first field of the next group waits for the last steps of the group, not for the root.
    ///
    /// In a group, a field waits only for the root fetch of the field before it, so
    /// `merge_children_with_parents` puts their root fetches in one request, in order. The
    /// entity calls of the group run after that request: in
    /// `create { isExpensive } double: multiply(by: 2)`, `double` runs before `b` fetches
    /// `isExpensive`.
    ///
    /// The idea with grouping is here to validate two things:
    /// 1 - mutations fields are always running as sequence (serially), even if they are from different subgraphs
    /// 2 - grouping happens only if it's helping with efficiency (same subgraph)
    /// 3 - entity/requires calls are still happening within the same group
    ///
    /// Runs before any optimization. With the order in the graph, merges keep it like any other
    /// dependency, and nothing else has to know about mutations.
    #[instrument(level = "trace", skip_all)]
    pub(crate) fn turn_mutations_into_sequence(&mut self) -> Result<(), FetchGraphError> {
        if self.operation_kind != OperationKind::Mutation {
            return Ok(());
        }

        let root_index = self
            .root_index
            .ok_or(FetchGraphError::NonSingleRootStep(0))?;

        let mut fields = BTreeMap::<usize, Vec<NodeIndex>>::new();
        for edge_ref in self.children_of(root_index) {
            let step_index = edge_ref.target();
            let position = self
                .get_step_data(step_index)?
                .mutation_field_position
                .ok_or(FetchGraphError::MutationStepWithNoOrder)?;
            fields.entry(position).or_default().push(step_index);
        }

        // Consecutive fields whose root fetches go to one subgraph.
        let mut groups: Vec<Vec<Vec<NodeIndex>>> = Vec::new();
        let mut group_subgraph = None;
        for steps in fields.into_values() {
            let subgraph = self.single_subgraph(&steps)?;
            match groups.last_mut() {
                Some(group) if subgraph.is_some() && subgraph == group_subgraph => {
                    group.push(steps)
                }
                _ => {
                    group_subgraph = subgraph;
                    groups.push(vec![steps]);
                }
            }
        }

        for group in &groups {
            for pair in group.windows(2) {
                self.wait_for(root_index, &pair[0], &pair[1]);
            }
        }

        for pair in groups.windows(2) {
            let previous: Vec<NodeIndex> = pair[0].iter().flatten().copied().collect();
            let next = &pair[1][0];
            let next_steps = self.reachable_from(next);
            let last_steps: Vec<NodeIndex> = self
                .reachable_from(&previous)
                .into_iter()
                .filter(|step| self.children_of(*step).next().is_none())
                // A step both groups use can't wait for the second one.
                .filter(|step| !next_steps.contains(step))
                .collect();
            self.wait_for(root_index, &last_steps, next);
        }

        Ok(())
    }

    /// The steps of `next` wait for `previous` instead of the root. With nothing to wait for,
    /// they stay where they are.
    fn wait_for(&mut self, root_index: NodeIndex, previous: &[NodeIndex], next: &[NodeIndex]) {
        if previous.is_empty() {
            return;
        }
        for next_step in next {
            if let Some(edge) = self.graph.find_edge(root_index, *next_step) {
                self.remove_edge(edge);
            }
            for previous_step in previous {
                self.connect(*previous_step, *next_step);
            }
        }
    }

    /// The subgraph all `steps` go to, `None` when they go to more than one.
    fn single_subgraph(
        &self,
        steps: &[NodeIndex],
    ) -> Result<Option<SubgraphName>, FetchGraphError> {
        let mut subgraph: Option<&SubgraphName> = None;
        for step in steps {
            let service_name = &self.get_step_data(*step)?.service_name;
            match subgraph {
                Some(current) if current != service_name => return Ok(None),
                _ => subgraph = Some(service_name),
            }
        }
        Ok(subgraph.cloned())
    }

    fn reachable_from(&self, starts: &[NodeIndex]) -> HashSet<NodeIndex> {
        let mut reachable = HashSet::new();
        for start in starts {
            let mut bfs = Bfs::new(&self.graph, *start);
            while let Some(step) = bfs.next(&self.graph) {
                reachable.insert(step);
            }
        }
        reachable
    }
}
