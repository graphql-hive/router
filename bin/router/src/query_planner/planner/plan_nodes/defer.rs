//! `@defer` in plans: which part each fetch is for (the plan's schedule and the executor use it),
//! and the `Defer` node shown when a plan is exposed.

use serde::ser::{Serialize, Serializer};

use super::dependencies::{plan_fetches, PlanSchedule};
use super::{
    DeferDependency, DeferNode, DeferPrimary, DeferredNode, FetchId, FlattenNodePath, ParallelNode,
    PlanNode, PlanState, QueryPlan, SequenceNode,
};
use crate::query_planner::ast::defer::{DeferDisplay, DeferId, DeferLayout, DeferPart};

/// The deferred part a fetch is for, `None` for the primary part and for nodes that aren't
/// fetches.
pub fn part_of_fetch<S: PlanState>(node: &PlanNode<S>) -> Option<DeferId> {
    match node {
        PlanNode::Fetch(fetch) => fetch.defer,
        PlanNode::Flatten(flatten) => part_of_fetch(&flatten.node),
        PlanNode::BatchFetch(batch) => batch.defer,
        _ => None,
    }
}

/// Whether `part` is `outer` or nested in it. Everything is in the primary part, `None`.
pub fn is_within(parts: &[DeferPart], mut part: Option<DeferId>, outer: Option<DeferId>) -> bool {
    loop {
        if part == outer {
            return true;
        }
        match part {
            Some(id) => part = parts[id.index()].parent,
            None => return false,
        }
    }
}

/// A plan node serialized with its fetch ids. Fetch ids are left out of plans, a `Defer` node
/// shows them for what's in it: its `depends` point at them.
pub(crate) struct WithIds<'p, S: PlanState>(pub &'p PlanNode<S>);

/// For `#[serde(serialize_with)]` on the nodes in a `Defer` node.
pub(crate) fn with_ids<S: PlanState, Se: Serializer>(
    node: &Option<Box<PlanNode<S>>>,
    serializer: Se,
) -> Result<Se::Ok, Se::Error> {
    node.as_deref().map(WithIds).serialize(serializer)
}

impl<S: PlanState> Serialize for WithIds<'_, S> {
    fn serialize<Se: Serializer>(&self, serializer: Se) -> Result<Se::Ok, Se::Error> {
        // the same JSON as `PlanNode`'s, `kind` first, with `id` added to fetches
        #[derive(serde::Serialize)]
        struct Fetch<'n, T: Serialize> {
            kind: &'static str,
            id: FetchId,
            #[serde(flatten)]
            node: &'n T,
        }
        #[derive(serde::Serialize)]
        #[serde(bound = "")]
        struct Flatten<'n, S: PlanState> {
            kind: &'static str,
            path: &'n FlattenNodePath,
            node: WithIds<'n, S>,
        }
        #[derive(serde::Serialize)]
        #[serde(bound = "")]
        struct Nodes<'n, S: PlanState> {
            kind: &'static str,
            nodes: Vec<WithIds<'n, S>>,
        }
        fn nodes<'n, S: PlanState>(kind: &'static str, nodes: &'n [PlanNode<S>]) -> Nodes<'n, S> {
            Nodes {
                kind,
                nodes: nodes.iter().map(WithIds).collect(),
            }
        }
        match self.0 {
            PlanNode::Fetch(fetch) => Fetch {
                kind: "Fetch",
                id: fetch.id,
                node: fetch.as_ref(),
            }
            .serialize(serializer),
            PlanNode::BatchFetch(batch) => Fetch {
                kind: "BatchFetch",
                id: batch.id,
                node: batch.as_ref(),
            }
            .serialize(serializer),
            PlanNode::Flatten(flatten) => Flatten {
                kind: "Flatten",
                path: &flatten.path,
                node: WithIds(&flatten.node),
            }
            .serialize(serializer),
            PlanNode::Sequence(sequence) => {
                nodes("Sequence", &sequence.nodes).serialize(serializer)
            }
            PlanNode::Parallel(parallel) => {
                nodes("Parallel", &parallel.nodes).serialize(serializer)
            }
            // a nested `Defer` node shows its own ids, a subscription can't be deferred
            node @ (PlanNode::Defer(_) | PlanNode::Subscription(_)) => node.serialize(serializer),
        }
    }
}

fn fetch_id(node: &PlanNode) -> Option<FetchId> {
    match node {
        PlanNode::Fetch(fetch) => Some(fetch.id),
        PlanNode::Flatten(flatten) => fetch_id(&flatten.node),
        PlanNode::BatchFetch(batch) => Some(batch.id),
        _ => None,
    }
}

/// The part of `node` that belongs to `want`, `Sequence`/`Parallel` kept around it.
fn part_of(node: &PlanNode, want: Option<DeferId>) -> Option<PlanNode> {
    // a block left with one node is just that node
    let parts = |nodes: &[PlanNode], wrap: fn(Vec<PlanNode>) -> PlanNode| {
        let mut nodes: Vec<PlanNode> = nodes
            .iter()
            .filter_map(|child| part_of(child, want))
            .collect();
        match nodes.len() {
            0 => None,
            1 => nodes.pop(),
            _ => Some(wrap(nodes)),
        }
    };
    match node {
        PlanNode::Sequence(sequence) => parts(&sequence.nodes, |nodes| {
            PlanNode::Sequence(SequenceNode { nodes })
        }),
        PlanNode::Parallel(parallel) => parts(&parallel.nodes, |nodes| {
            PlanNode::Parallel(ParallelNode { nodes })
        }),
        node => (part_of_fetch(node) == want).then(|| without_part(node)),
    }
}

/// A fetch without its part, the `Defer` node around it says it already.
fn without_part(node: &PlanNode) -> PlanNode {
    let mut node = node.clone();
    match &mut node {
        PlanNode::Fetch(fetch) => fetch.defer = None,
        PlanNode::Flatten(flatten) => {
            if let PlanNode::Fetch(fetch) = flatten.node.as_mut() {
                fetch.defer = None;
            }
        }
        PlanNode::BatchFetch(batch) => batch.defer = None,
        _ => {}
    }
    node
}

/// A fetch of the plan, with its part and the fetches it waits for (by position).
struct Leaf<'p> {
    node: &'p PlanNode,
    part: Option<DeferId>,
    waits_for: Vec<usize>,
}

/// `plan` as it's shown when exposed: the deferred fetches in `Defer` nodes. `layout` comes
/// from `prepare_for_planning`.
pub fn defer_display_plan(plan: &QueryPlan, layout: &DeferLayout) -> QueryPlan {
    let Some(root) = plan.node() else {
        return plan.clone();
    };
    let schedule = plan.schedule();
    let mut leaves: Vec<Leaf> = plan_fetches(root)
        .into_iter()
        .map(|node| Leaf {
            node,
            part: part_of_fetch(node),
            waits_for: Vec::new(),
        })
        .collect();
    for index in 0..leaves.len() {
        for &dependent in schedule.dependents(index) {
            leaves[dependent as usize].waits_for.push(index);
        }
    }

    fn build(
        want: Option<DeferId>,
        root: &PlanNode,
        leaves: &[Leaf],
        parts: &[DeferPart],
        display: &DeferDisplay,
    ) -> Option<PlanNode> {
        let own = part_of(root, want);
        let children: Vec<(DeferId, &DeferPart)> = parts
            .iter()
            .enumerate()
            .map(|(index, part)| (DeferId::from_index(index), part))
            .filter(|(_, part)| part.parent == want)
            .collect();
        if children.is_empty() {
            return own;
        }
        let parts = children
            .into_iter()
            .map(|(id, child)| {
                // the fetches outside this part that its fetches wait for: the part around it's,
                // or any further out, with the part each one is in
                let mut depends: Vec<(FetchId, Option<DeferId>)> = leaves
                    .iter()
                    .filter(|leaf| leaf.part == Some(id))
                    .flat_map(|leaf| leaf.waits_for.iter())
                    .map(|dep| &leaves[*dep])
                    .filter(|dep| !is_within(parts, dep.part, Some(id)))
                    .filter_map(|dep| Some((fetch_id(dep.node)?, dep.part)))
                    .collect();
                depends.sort_unstable();
                depends.dedup();
                let node = build(Some(id), root, leaves, parts, display);
                // a part with deferred parts of its own shows it on its `Primary`
                let subselection = match node {
                    Some(PlanNode::Defer(_)) => None,
                    _ => display.parts[id.index()].clone(),
                };
                DeferredNode {
                    depends: depends
                        .into_iter()
                        .map(|(id, part)| DeferDependency {
                            id: id.to_string(),
                            defer_label: part.and_then(|part| parts[part.index()].label.clone()),
                        })
                        .collect(),
                    label: child.label.clone(),
                    query_path: child.path.iter().map(ToString::to_string).collect(),
                    subselection,
                    node: node.map(Box::new),
                }
            })
            .collect();
        Some(PlanNode::Defer(Box::new(DeferNode {
            primary: DeferPrimary {
                subselection: match want {
                    Some(id) => display.parts[id.index()].clone(),
                    None => display.primary.clone(),
                },
                node: own.map(Box::new),
            },
            deferred: parts,
        })))
    }

    // only shown, never run, so it doesn't need a schedule
    QueryPlan::with_schedule(
        build(None, root, &leaves, &layout.parts, &layout.display),
        PlanSchedule::default(),
    )
}
