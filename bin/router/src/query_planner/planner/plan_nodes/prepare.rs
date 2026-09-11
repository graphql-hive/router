use super::*;

impl QueryPlan<Planning> {
    /// Drops the parsed documents the planner used,
    /// and keeps the operation text that execution sends.
    pub fn into_executable(self) -> QueryPlan {
        QueryPlan {
            kind: self.kind,
            node: self.node.map(PlanNode::into_executable),
        }
    }
}

impl FetchNode<Planning> {
    fn into_executable(self) -> FetchNode {
        FetchNode {
            id: self.id,
            service_name: self.service_name,
            variable_usages: self.variable_usages,
            operation_kind: self.operation_kind,
            operation: self.operation.operation,
            custom_scalar_paths: self.custom_scalar_paths,
            requires: self.requires,
            input_rewrites: self.input_rewrites,
            output_rewrites: self.output_rewrites,
        }
    }
}

impl PlanNode<Planning> {
    fn into_executable(self) -> PlanNode {
        fn boxed(node: PlanNode<Planning>) -> Box<PlanNode> {
            Box::new(node.into_executable())
        }
        match self {
            Self::Fetch(fetch) => PlanNode::Fetch(Box::new(fetch.into_executable())),
            Self::BatchFetch(batch) => PlanNode::BatchFetch(Box::new(BatchFetchNode {
                id: batch.id,
                service_name: batch.service_name,
                variable_usages: batch.variable_usages,
                operation_kind: batch.operation_kind,
                operation: batch.operation.operation,
                custom_scalar_paths: batch.custom_scalar_paths,
                entity_batch: batch.entity_batch,
            })),
            Self::Flatten(flatten) => PlanNode::Flatten(FlattenNode {
                path: flatten.path,
                node: boxed(*flatten.node),
            }),
            Self::Sequence(sequence) => PlanNode::Sequence(SequenceNode {
                nodes: sequence
                    .nodes
                    .into_iter()
                    .map(Self::into_executable)
                    .collect(),
            }),
            Self::Parallel(parallel) => PlanNode::Parallel(ParallelNode {
                nodes: parallel
                    .nodes
                    .into_iter()
                    .map(Self::into_executable)
                    .collect(),
            }),
            Self::Condition(condition) => PlanNode::Condition(ConditionNode {
                condition: condition.condition,
                if_clause: condition.if_clause.map(|node| boxed(*node)),
                else_clause: condition.else_clause.map(|node| boxed(*node)),
            }),
            Self::Subscription(subscription) => {
                PlanNode::Subscription(Box::new(SubscriptionNode {
                    primary: subscription.primary.into_executable(),
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
