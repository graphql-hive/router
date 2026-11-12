use super::*;

impl QueryPlan<Planning> {
    /// Drops the parsed documents the planner used,
    /// and keeps the operation text that execution sends.
    pub fn into_executable(self, supergraph: &SupergraphState) -> QueryPlan {
        // the nodes keep their shape, so the schedule stays right
        let schedule = self.schedule().clone();
        QueryPlan::with_schedule(
            self.node.map(|node| node.into_executable(supergraph)),
            schedule,
        )
    }
}

impl FetchNode<Planning> {
    fn into_executable(self, supergraph: &SupergraphState) -> FetchNode {
        FetchNode {
            id: self.id,
            service_name: self.service_name,
            variable_usages: self.variable_usages,
            operation_kind: self.operation_kind,
            operation: self
                .operation
                .into_executable(supergraph, self.custom_scalar_paths.as_ref()),
            custom_scalar_paths: self.custom_scalar_paths,
            requires: self.requires,
            planner_requires: (),
            input_rewrites: self.input_rewrites,
            output_rewrites: self.output_rewrites,
            context: self.context,
            run_if: self.run_if,
            depends_on: (),
            entity_reuse: self.entity_reuse,
        }
    }
}

impl PlanNode<Planning> {
    fn into_executable(self, supergraph: &SupergraphState) -> PlanNode {
        let boxed = |node: PlanNode<Planning>| Box::new(node.into_executable(supergraph));
        match self {
            Self::Fetch(fetch) => PlanNode::Fetch(Box::new(fetch.into_executable(supergraph))),
            Self::BatchFetch(batch) => PlanNode::BatchFetch(Box::new(BatchFetchNode {
                id: batch.id,
                service_name: batch.service_name,
                variable_usages: batch.variable_usages,
                operation_kind: batch.operation_kind,
                operation: batch
                    .operation
                    .into_executable(supergraph, batch.custom_scalar_paths.as_ref()),
                custom_scalar_paths: batch.custom_scalar_paths,
                entity_batch: batch.entity_batch,
                depends_on: (),
                completes: (),
            })),
            Self::Flatten(flatten) => PlanNode::Flatten(FlattenNode {
                path: flatten.path,
                node: boxed(*flatten.node),
            }),
            Self::Sequence(sequence) => PlanNode::Sequence(SequenceNode {
                nodes: sequence
                    .nodes
                    .into_iter()
                    .map(|node| node.into_executable(supergraph))
                    .collect(),
            }),
            Self::Parallel(parallel) => PlanNode::Parallel(ParallelNode {
                nodes: parallel
                    .nodes
                    .into_iter()
                    .map(|node| node.into_executable(supergraph))
                    .collect(),
            }),
            Self::Subscription(subscription) => {
                PlanNode::Subscription(Box::new(SubscriptionNode {
                    primary: subscription.primary.into_executable(supergraph),
                }))
            }
            Self::Defer(defer) => PlanNode::Defer(Box::new(DeferNode {
                primary: DeferPrimary {
                    subselection: defer.primary.subselection,
                    node: defer.primary.node.map(|node| boxed(*node)),
                },
                deferred: defer
                    .deferred
                    .into_iter()
                    .map(|node| DeferredNode {
                        depends: node.depends,
                        label: node.label,
                        query_path: node.query_path,
                        subselection: node.subselection,
                        node: node.node.map(|node| boxed(*node)),
                    })
                    .collect(),
            })),
        }
    }
}
