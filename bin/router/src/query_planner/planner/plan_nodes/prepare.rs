use super::*;

impl QueryPlan<Planning> {
    /// Drops the parsed documents the planner used, and keeps the operation text that execution
    /// sends.
    ///
    /// Call this after demand control has compiled its costs, and before the plan is cached. The
    /// parsed documents are most of the size of a cached plan, and nothing on the execution path
    /// reads them.
    ///
    /// An executable plan cannot be passed back to planning code.
    /// ```compile_fail,E0308
    /// use hive_router::query_planner::planner::plan_nodes::{QueryPlan, Planning};
    /// fn optimize(_: QueryPlan<Planning>) {}
    /// let planning = QueryPlan::<Planning> { kind: "QueryPlan", node: None };
    /// optimize(planning.into_executable());
    /// ```
    /// Converting a plan also consumes the original, so it cannot be used again.
    /// ```compile_fail,E0382
    /// use hive_router::query_planner::planner::plan_nodes::{QueryPlan, Planning};
    /// let planning = QueryPlan::<Planning> { kind: "QueryPlan", node: None };
    /// let executable = planning.into_executable();
    /// let still_planning = planning;
    /// ```
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
            Self::Fetch(fetch) => PlanNode::Fetch(fetch.into_executable()),
            Self::BatchFetch(batch) => PlanNode::BatchFetch(BatchFetchNode {
                id: batch.id,
                service_name: batch.service_name,
                variable_usages: batch.variable_usages,
                operation_kind: batch.operation_kind,
                operation: batch.operation.operation,
                custom_scalar_paths: batch.custom_scalar_paths,
                entity_batch: batch.entity_batch,
            }),
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
            Self::Subscription(subscription) => PlanNode::Subscription(SubscriptionNode {
                primary: subscription.primary.into_executable(),
            }),
            Self::Defer(defer) => PlanNode::Defer(DeferNode {
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
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query_planner::ast::document::Document;
    use crate::query_planner::ast::selection_set::FieldSelection;

    /// A minimal `{ id }` document, enough to give a fetch an operation and a `requires`.
    fn document() -> Document {
        Document {
            operation: OperationDefinition {
                name: None,
                operation_kind: Some(OperationKind::Query),
                selection_set: SelectionSet {
                    items: vec![SelectionItem::Field(FieldSelection {
                        name: "id".into(),
                        ..Default::default()
                    })],
                },
                variable_definitions: None,
            },
            fragments: Vec::new(),
        }
    }

    fn fetch() -> FetchNode<Planning> {
        let document = document();
        let requires = document.operation.selection_set.clone();
        FetchNode {
            id: 17,
            service_name: "products".into(),
            variable_usages: Some(BTreeSet::from(["representations".into()])),
            operation_kind: Some(OperationKind::Query),
            operation: PlanningFetchOperation::from_anonymous_operation(document),
            custom_scalar_paths: Some(CustomScalarPaths::default()),
            requires: Some(requires),
            input_rewrites: Some(vec![]),
            output_rewrites: Some(vec![]),
        }
    }

    fn leaf() -> PlanNode<Planning> {
        PlanNode::Fetch(fetch())
    }

    /// Collects what identifies each fetch after conversion: its id, the memory address of its
    /// operation text, its hash, and the position where its name is written. The address is
    /// checked because the text must be moved, not rendered again.
    fn operation_metadata<S: PlanState>(
        node: &PlanNode<S>,
        out: &mut Vec<(i64, usize, u64, usize)>,
    ) {
        let mut record = |id, operation: &S::Operation| {
            let operation = operation.as_ref();
            out.push((
                id,
                operation.document_str.as_ptr() as usize,
                operation.hash,
                operation.name_write_position,
            ));
        };
        match node {
            PlanNode::Fetch(fetch) => record(fetch.id, &fetch.operation),
            PlanNode::BatchFetch(fetch) => record(fetch.id, &fetch.operation),
            PlanNode::Subscription(subscription) => {
                record(subscription.primary.id, &subscription.primary.operation)
            }
            PlanNode::Flatten(flatten) => operation_metadata(&flatten.node, out),
            PlanNode::Sequence(sequence) => sequence
                .nodes
                .iter()
                .for_each(|node| operation_metadata(node, out)),
            PlanNode::Parallel(parallel) => parallel
                .nodes
                .iter()
                .for_each(|node| operation_metadata(node, out)),
            PlanNode::Condition(condition) => {
                for node in [
                    condition.if_clause.as_deref(),
                    condition.else_clause.as_deref(),
                ]
                .into_iter()
                .flatten()
                {
                    operation_metadata(node, out);
                }
            }
            PlanNode::Defer(defer) => {
                if let Some(node) = &defer.primary.node {
                    operation_metadata(node, out);
                }
                for node in defer.deferred.iter().filter_map(|node| node.node.as_ref()) {
                    operation_metadata(node, out);
                }
            }
        }
    }

    #[test]
    fn preparation_preserves_every_node_kind_and_moves_operation_text() {
        let fetch = fetch();
        let batch = PlanNode::BatchFetch(BatchFetchNode {
            id: fetch.id,
            service_name: fetch.service_name,
            variable_usages: fetch.variable_usages,
            operation_kind: fetch.operation_kind,
            operation: fetch.operation,
            custom_scalar_paths: fetch.custom_scalar_paths,
            entity_batch: EntityBatch { aliases: vec![] },
        });
        let planning = QueryPlan {
            kind: "QueryPlan",
            node: Some(PlanNode::Sequence(SequenceNode {
                nodes: vec![
                    leaf(),
                    batch,
                    PlanNode::Subscription(SubscriptionNode {
                        primary: self::fetch(),
                    }),
                    PlanNode::Flatten(FlattenNode {
                        path: FlattenNodePath(vec![]),
                        node: Box::new(leaf()),
                    }),
                    PlanNode::Parallel(ParallelNode {
                        nodes: vec![leaf()],
                    }),
                    PlanNode::Condition(ConditionNode {
                        condition: "enabled".into(),
                        if_clause: Some(Box::new(leaf())),
                        else_clause: Some(Box::new(leaf())),
                    }),
                    PlanNode::Condition(ConditionNode {
                        condition: "empty".into(),
                        if_clause: None,
                        else_clause: None,
                    }),
                    PlanNode::Defer(DeferNode {
                        primary: DeferPrimary {
                            subselection: Some("{ id }".into()),
                            node: Some(Box::new(leaf())),
                        },
                        deferred: vec![
                            DeferredNode {
                                depends: vec![DeferDependency {
                                    id: "17".into(),
                                    defer_label: Some("later".into()),
                                }],
                                label: Some("later".into()),
                                query_path: vec!["products".into()],
                                subselection: Some("{ id }".into()),
                                node: Some(Box::new(leaf())),
                            },
                            DeferredNode {
                                depends: vec![],
                                label: None,
                                query_path: vec![],
                                subselection: None,
                                node: None,
                            },
                        ],
                    }),
                    PlanNode::Defer(DeferNode {
                        primary: DeferPrimary {
                            subselection: None,
                            node: None,
                        },
                        deferred: vec![],
                    }),
                ],
            })),
        };
        let json = serde_json::to_string(&planning).unwrap();
        let mut before = vec![];
        operation_metadata(planning.node.as_ref().unwrap(), &mut before);
        assert_eq!(before.len(), 9, "every fetch-carrying node kind is covered");

        let executable = planning.into_executable();
        let mut after = vec![];
        operation_metadata(executable.node.as_ref().unwrap(), &mut after);
        assert_eq!(
            before, after,
            "fetch IDs, hashes, insertion positions and string allocations must survive"
        );
        assert_eq!(
            json,
            serde_json::to_string(&executable).unwrap(),
            "lowering must not change the wire form"
        );

        assert!(QueryPlan::<Planning> {
            kind: "QueryPlan",
            node: None
        }
        .into_executable()
        .node
        .is_none());
    }
}
