use super::*;
use crate::executor::utils::traverse::PathTrie;
use std::sync::Arc;

impl QueryPlan<Planning> {
    /// Drops the parsed documents the planner used,
    /// and keeps the operation text that execution sends.
    pub fn into_executable(self, supergraph: &SupergraphState) -> QueryPlan {
        let mut prepare = Prepare {
            supergraph,
            steps: Default::default(),
            reuses: Default::default(),
        };
        // the nodes keep their shape, so the schedule stays right
        let schedule = self.schedule().clone();
        QueryPlan::with_schedule(
            self.node.map(|node| node.into_executable(&mut prepare)),
            schedule,
        )
    }
}

/// What making a plan executable keeps track of. The executable plan stays in the cache, so it
/// keeps its paths only as tries, and they share one copy of every name.
struct Prepare<'s> {
    supergraph: &'s SupergraphState,
    steps: SharedSteps,
    /// Fetches share their entity reuse, so each one is made executable once. Keyed by the
    /// planner's one, which we hold on to so no other one gets its address while this runs.
    reuses: ahash::HashMap<*const EntityReuse, (Arc<EntityReuse>, Arc<EntityReuse>)>,
}

impl Prepare<'_> {
    /// The paths are final once the plan is, so that's when their tries get built. Execution
    /// only walks the trie, so the executable one keeps no `sources`.
    fn reuse(&mut self, reuse: Option<Arc<EntityReuse>>) -> Option<Arc<EntityReuse>> {
        let reuse = reuse?;
        let (_, executable) = self.reuses.entry(Arc::as_ptr(&reuse)).or_insert_with(|| {
            let sources = reuse.sources.iter().map(FlattenNodePath::as_slice);
            let executable = EntityReuse {
                sources: Vec::new(),
                copy: reuse.copy.clone(),
                trie: PathTrie::sharing(sources, &mut self.steps).into(),
            };
            (reuse, Arc::new(executable))
        });
        Some(Arc::clone(executable))
    }

    /// One trie for the merge paths of every alias, one alias after another
    fn entity_batch(&mut self, entity_batch: EntityBatch<Planning>) -> EntityBatch {
        let merge_paths = entity_batch.aliases.iter().flat_map(|alias| {
            alias
                .merge_paths
                .iter()
                .map(|merge_path| merge_path.as_slice())
        });
        let trie = Arc::new(PathTrie::sharing(merge_paths, &mut self.steps));
        let mut start = 0;
        let aliases = entity_batch
            .aliases
            .into_iter()
            .map(|alias| {
                let paths = start..start + alias.merge_paths.len();
                start = paths.end;
                EntityBatchAlias {
                    alias: alias.alias,
                    representations_variable_name: alias.representations_variable_name,
                    merge_paths: TriePaths::new(Arc::clone(&trie), paths),
                    requires: alias.requires,
                    input_rewrites: alias.input_rewrites,
                    output_rewrites: alias.output_rewrites,
                    context: alias.context,
                    run_if: alias.run_if,
                    entity_reuse: self.reuse(alias.entity_reuse),
                }
            })
            .collect();
        EntityBatch { aliases }
    }
}

impl FetchNode<Planning> {
    fn into_executable(self, prepare: &mut Prepare) -> FetchNode {
        FetchNode {
            id: self.id,
            service_name: self.service_name,
            variable_usages: self.variable_usages,
            operation_kind: self.operation_kind,
            operation: self
                .operation
                .into_executable(prepare.supergraph, self.custom_scalar_paths.as_ref()),
            custom_scalar_paths: self.custom_scalar_paths,
            requires: self.requires,
            planner_requires: (),
            input_rewrites: self.input_rewrites,
            output_rewrites: self.output_rewrites,
            context: self.context,
            run_if: self.run_if,
            depends_on: (),
            entity_reuse: prepare.reuse(self.entity_reuse),
        }
    }
}

impl EntityReuse {
    pub fn build_trie(&self) {
        self.trie
            .get_or_init(|| PathTrie::new(self.sources.iter().map(FlattenNodePath::as_slice)));
    }
}

impl EntityBatch {
    /// For a batch that wasn't made executable from a plan, like one built by hand: puts the
    /// merge paths of every alias in one trie, and builds the tries of where each alias's
    /// entities might already be.
    pub fn build_tries(&mut self) {
        let merge_paths: Vec<FlattenNodePath> = self
            .aliases
            .iter()
            .flat_map(|alias| alias.merge_paths.to_paths().into_owned())
            .collect();
        let trie = Arc::new(PathTrie::new(
            merge_paths.iter().map(FlattenNodePath::as_slice),
        ));
        let mut start = 0;
        for alias in &mut self.aliases {
            if let Some(reuse) = &alias.entity_reuse {
                reuse.build_trie();
            }
            let paths = start..start + alias.merge_paths.len();
            start = paths.end;
            alias.merge_paths = TriePaths::new(Arc::clone(&trie), paths);
        }
    }
}

impl PlanNode<Planning> {
    fn into_executable(self, prepare: &mut Prepare) -> PlanNode {
        match self {
            Self::Fetch(fetch) => PlanNode::Fetch(Box::new(fetch.into_executable(prepare))),
            Self::BatchFetch(batch) => PlanNode::BatchFetch(Box::new(BatchFetchNode {
                id: batch.id,
                service_name: batch.service_name,
                variable_usages: batch.variable_usages,
                operation_kind: batch.operation_kind,
                operation: batch
                    .operation
                    .into_executable(prepare.supergraph, batch.custom_scalar_paths.as_ref()),
                custom_scalar_paths: batch.custom_scalar_paths,
                entity_batch: prepare.entity_batch(batch.entity_batch),
                depends_on: (),
                completes: (),
            })),
            Self::Flatten(flatten) => PlanNode::Flatten(FlattenNode {
                path: TriePath::new(flatten.path.as_slice(), &mut prepare.steps),
                node: Box::new(flatten.node.into_executable(prepare)),
            }),
            Self::Sequence(sequence) => PlanNode::Sequence(SequenceNode {
                nodes: sequence
                    .nodes
                    .into_iter()
                    .map(|node| node.into_executable(prepare))
                    .collect(),
            }),
            Self::Parallel(parallel) => PlanNode::Parallel(ParallelNode {
                nodes: parallel
                    .nodes
                    .into_iter()
                    .map(|node| node.into_executable(prepare))
                    .collect(),
            }),
            Self::Subscription(subscription) => {
                PlanNode::Subscription(Box::new(SubscriptionNode {
                    primary: subscription.primary.into_executable(prepare),
                }))
            }
            Self::Defer(defer) => PlanNode::Defer(Box::new(DeferNode {
                primary: DeferPrimary {
                    subselection: defer.primary.subselection,
                    node: defer
                        .primary
                        .node
                        .map(|node| Box::new(node.into_executable(prepare))),
                },
                deferred: defer
                    .deferred
                    .into_iter()
                    .map(|node| DeferredNode {
                        depends: node.depends,
                        label: node.label,
                        query_path: node.query_path,
                        subselection: node.subselection,
                        node: node
                            .node
                            .map(|node| Box::new(node.into_executable(prepare))),
                    })
                    .collect(),
            })),
        }
    }
}
