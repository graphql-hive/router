//! Moving entity calls to a later wave so more of them share a request.
//!
//! A step runs in the first wave after its parents, but it can often run later without making
//! the plan longer: anything up to the wave before its earliest child, or the last wave. Entity
//! calls to the same subgraph in the same wave become one request, so for each subgraph we look
//! for calls whose windows meet and put them in the earliest wave they can all run in. A call
//! that would end up alone stays where it was, and so do calls that would share a request
//! anyway: moving them would only make them start later.
//!
//! With entity reuse on, a later call to a subgraph can copy what an earlier one already asked it
//! for, and skip the subgraph. Those two never go in one request, and the earlier one never
//! moves to where the later one runs: there'd be nothing to copy from yet.

use std::{cmp::Reverse, collections::BTreeSet};

use ahash::{HashMap, HashMapExt};
use petgraph::{
    algo::toposort,
    graph::NodeIndex,
    visit::{EdgeRef, NodeIndexable},
    Direction,
};

use crate::query_planner::{
    ast::selection_set::SelectionSet,
    planner::{
        error::QueryPlanError,
        fetch::{fetch_graph::FetchGraph, state::MultiTypeFetchStep},
    },
    state::supergraph_state::OperationKind,
};

use super::entity_reuse::{covers, wanted};

type MultiTypeFetchGraph = FetchGraph<MultiTypeFetchStep>;

/// The wave of every step, by index: the earliest one, or a later one where it joins other calls
/// to its subgraph.
pub(super) fn schedule_entity_batches(
    fetch_graph: &MultiTypeFetchGraph,
    root: NodeIndex,
    entity_reuse: bool,
) -> Result<Vec<usize>, QueryPlanError> {
    let input = SchedulingInput::new(fetch_graph, root)?;
    let mut reuse = ReuseIndex::new(&input, entity_reuse);
    let baseline = Schedule::new(&input);

    let mut schedule = baseline.clone();
    for steps in input
        .movable_by_subgraph
        .iter()
        .filter(|steps| steps.len() > 1)
    {
        let mut candidates = BatchCandidates::new(steps, &reuse, &schedule);
        while let Some(batch_move) = candidates.next_move(&schedule, &mut reuse) {
            schedule.apply_move(&input, &batch_move);
            candidates.refresh(&schedule);
        }
    }

    // a move also moves what waits for it, and that can take calls of other subgraphs out of
    // requests they shared. Those usually get merged again by later moves, but when the moves
    // don't save a request in the end, the plan is better off without any: they'd only make calls
    // start later.
    if input.batch_cost_estimate(&schedule) >= input.batch_cost_estimate(&baseline) {
        return Ok(baseline.earliest);
    }
    Ok(schedule.earliest)
}

/// The steps of a fetch graph, and the entity calls that can move.
struct SchedulingInput<'a> {
    graph: &'a MultiTypeFetchGraph,
    root: NodeIndex,
    /// every step but the root, parents first
    topological_order: Vec<NodeIndex>,
    /// the calls that can move, per subgraph, in `topological_order`. The subgraphs with the most
    /// calls come first, they have the most to share.
    movable_by_subgraph: Vec<Vec<NodeIndex>>,
}

impl<'a> SchedulingInput<'a> {
    fn new(graph: &'a MultiTypeFetchGraph, root: NodeIndex) -> Result<Self, QueryPlanError> {
        let topological_order: Vec<NodeIndex> = toposort(&graph.graph, None)
            .map_err(|_| QueryPlanError::Internal("the fetch graph has a cycle".to_string()))?
            .into_iter()
            .filter(|&index| index != root)
            .collect();
        // only entity calls of a query move: mutations keep their order, root fetches don't batch
        let movable = |index: NodeIndex| {
            let step = &graph.graph[index];
            graph.operation_kind == OperationKind::Query
                && step.is_entity_call()
                && step.context_producers.is_empty()
                && step.context_rewrites.is_none()
        };
        let mut movable_by_subgraph: Vec<(&str, Vec<NodeIndex>)> = Vec::new();
        for index in topological_order
            .iter()
            .copied()
            .filter(|&index| movable(index))
        {
            let name = graph.graph[index].service_name.0.as_str();
            match movable_by_subgraph
                .iter_mut()
                .find(|(subgraph, _)| *subgraph == name)
            {
                Some((_, steps)) => steps.push(index),
                None => movable_by_subgraph.push((name, vec![index])),
            }
        }
        movable_by_subgraph.sort_by_key(|(_, steps)| Reverse(steps.len()));
        Ok(Self {
            graph,
            root,
            topological_order,
            movable_by_subgraph: movable_by_subgraph
                .into_iter()
                .map(|(_, steps)| steps)
                .collect(),
        })
    }

    fn bound(&self) -> usize {
        self.graph.graph.node_bound()
    }

    /// Roughly the requests the calls make: one per subgraph and wave it has calls in. It's not
    /// exact, a batch with too many aliases gets split later on.
    fn batch_cost_estimate(&self, schedule: &Schedule) -> usize {
        self.movable_by_subgraph
            .iter()
            .map(|steps| {
                let waves: BTreeSet<usize> =
                    steps.iter().map(|&step| schedule.earliest(step)).collect();
                waves.len()
            })
            .sum()
    }
}

/// When each step runs: in the first wave it can, unless it's pinned to a later one. Also the
/// last wave it could run in without making the plan longer. All by step index.
#[derive(Clone)]
struct Schedule {
    pinned: Vec<Option<usize>>,
    earliest: Vec<usize>,
    latest: Vec<usize>,
}

impl Schedule {
    fn new(input: &SchedulingInput) -> Self {
        let mut schedule = Self {
            pinned: vec![None; input.bound()],
            earliest: Vec::new(),
            latest: Vec::new(),
        };
        schedule.recompute_windows(input);
        schedule
    }

    fn earliest(&self, step: NodeIndex) -> usize {
        self.earliest[step.index()]
    }

    fn latest(&self, step: NodeIndex) -> usize {
        self.latest[step.index()]
    }

    fn apply_move(&mut self, input: &SchedulingInput, batch_move: &BatchMove) {
        for step in &batch_move.steps {
            self.pinned[step.index()] = Some(batch_move.wave);
        }
        self.recompute_windows(input);
    }

    fn recompute_windows(&mut self, input: &SchedulingInput) {
        let graph = &input.graph.graph;
        self.earliest = vec![0; input.bound()];
        for &index in &input.topological_order {
            let after_parents = graph
                .edges_directed(index, Direction::Incoming)
                .filter(|edge| edge.source() != input.root)
                .map(|edge| self.earliest(edge.source()) + 1)
                .max()
                .unwrap_or(0);
            self.earliest[index.index()] =
                self.pinned[index.index()].unwrap_or(0).max(after_parents);
        }
        let last = input
            .topological_order
            .iter()
            .map(|&index| self.earliest(index))
            .max()
            .unwrap_or(0);
        self.latest = vec![last; input.bound()];
        for &index in input.topological_order.iter().rev() {
            let before_children = graph
                .edges_directed(index, Direction::Outgoing)
                .map(|edge| self.latest(edge.target()).saturating_sub(1))
                .min()
                .unwrap_or(last);
            self.latest[index.index()] = self.pinned[index.index()]
                .unwrap_or(before_children)
                .min(before_children);
        }
    }
}

/// Calls to move to one wave, where they share a request.
struct BatchMove {
    steps: Vec<NodeIndex>,
    wave: usize,
}

/// A group of calls asking for the same thing.
type SelectionGroupId = usize;

/// With entity reuse on, `b` may copy from `a` instead of asking the subgraph: they're calls to
/// the same subgraph for the same type, and `a` asks for all `b` needs. Then `a` has to run
/// before `b`, in one request there's nothing to copy from yet. It's the same test entity reuse
/// uses to wait for an earlier call to the same subgraph.
///
/// There are many calls but few different selections, so calls asking for the same thing share
/// a `SelectionGroupId`, and whether one covers another is worked out once, for the pairs that
/// come up.
struct ReuseIndex<'a> {
    selections: Vec<(&'a str, &'a SelectionSet)>,
    /// by step index, `None` for a call that can't copy, or with entity reuse off
    group_of_step: Vec<Option<SelectionGroupId>>,
    covers: HashMap<(SelectionGroupId, SelectionGroupId), bool>,
}

impl<'a> ReuseIndex<'a> {
    fn new(input: &SchedulingInput<'a>, entity_reuse: bool) -> Self {
        let mut reuse = Self {
            selections: Vec::new(),
            group_of_step: vec![None; input.bound()],
            covers: HashMap::new(),
        };
        if !entity_reuse {
            return reuse;
        }
        let mut ids: HashMap<(&str, &SelectionSet), SelectionGroupId> = HashMap::new();
        for &step in input.movable_by_subgraph.iter().flatten() {
            let Some(selection) = wanted(&input.graph.graph[step]) else {
                continue;
            };
            let id = *ids.entry(selection).or_insert_with(|| {
                reuse.selections.push(selection);
                reuse.selections.len() - 1
            });
            reuse.group_of_step[step.index()] = Some(id);
        }
        reuse
    }

    /// Could a call asking for `copier` copy from one asking for `source`?
    fn covers(&mut self, source: SelectionGroupId, copier: SelectionGroupId) -> bool {
        let selections = &self.selections;
        *self.covers.entry((source, copier)).or_insert_with(|| {
            let ((source_type, source_fields), (copier_type, copier_fields)) =
                (selections[source], selections[copier]);
            source_type == copier_type && covers(source_fields, copier_fields)
        })
    }

    /// Calls of these groups never share a request. A selection covers itself, so calls of the
    /// same group never do either.
    fn conflict(&mut self, a: SelectionGroupId, b: SelectionGroupId) -> bool {
        self.covers(a, b) || self.covers(b, a)
    }
}

/// An entity call that could move. Calls are ordered by `subgraph_order`, their place in their
/// subgraph's calls, which come in plan order: node indexes don't follow it.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Candidate {
    subgraph_order: usize,
    step: NodeIndex,
    group: Option<SelectionGroupId>,
}

/// One subgraph's calls, swept by deadline: the last wave each can run in. A call is consumed
/// once it's been looked at, whether it moved or not.
struct BatchCandidates {
    all: Vec<Candidate>,
    /// by subgraph order
    consumed: Vec<bool>,
    /// the calls left, the one that has to run first at the end
    by_latest: Vec<Candidate>,
    /// the calls that aren't ready yet, the one that can run first at the end
    not_ready: Vec<Candidate>,
    /// the ready calls that can't copy, and the ones that can, by selection group
    ready: BTreeSet<Candidate>,
    ready_by_group: HashMap<SelectionGroupId, BTreeSet<Candidate>>,
    /// per selection group, the waves its calls run in, sorted
    group_waves: HashMap<SelectionGroupId, Vec<usize>>,
}

impl BatchCandidates {
    fn new(steps: &[NodeIndex], reuse: &ReuseIndex, schedule: &Schedule) -> Self {
        let all = steps
            .iter()
            .enumerate()
            .map(|(subgraph_order, &step)| Candidate {
                subgraph_order,
                step,
                group: reuse.group_of_step[step.index()],
            })
            .collect();
        let mut candidates = Self {
            all,
            consumed: vec![false; steps.len()],
            by_latest: Vec::new(),
            not_ready: Vec::new(),
            ready: BTreeSet::new(),
            ready_by_group: HashMap::new(),
            group_waves: HashMap::new(),
        };
        candidates.refresh(schedule);
        candidates
    }

    /// Starts over with the calls left, for when the schedule changed.
    fn refresh(&mut self, schedule: &Schedule) {
        self.by_latest = self
            .all
            .iter()
            .copied()
            .filter(|call| !self.consumed[call.subgraph_order])
            .collect();
        self.by_latest
            .sort_by_key(|call| Reverse((schedule.latest(call.step), call.step.index())));
        self.not_ready.clone_from(&self.by_latest);
        self.not_ready
            .sort_by_key(|call| Reverse(schedule.earliest(call.step)));
        self.ready.clear();
        self.ready_by_group.clear();

        self.group_waves.clear();
        for call in &self.all {
            if let Some(group) = call.group {
                let waves = self.group_waves.entry(group).or_default();
                waves.push(schedule.earliest(call.step));
            }
        }
        for waves in self.group_waves.values_mut() {
            waves.sort_unstable();
        }
    }

    /// The next calls to move: the call left with the earliest deadline, and the ready calls that
    /// can share a request with it, moved to the earliest wave they can all run in. Calls that
    /// would run alone, or share a request anyway, stay where they are.
    fn next_move(&mut self, schedule: &Schedule, reuse: &mut ReuseIndex) -> Option<BatchMove> {
        let earliest = |call: &Candidate| schedule.earliest(call.step);
        // the earliest wave all of `calls` can run in
        let earliest_shared = |calls: &[Candidate]| calls.iter().map(earliest).fold(0, usize::max);
        while let Some(first) = self.next_by_deadline() {
            // calls are taken by deadline, so every call left has a deadline at `deadline` or
            // later. Whether one can run in that wave only depends on its earliest wave then.
            let deadline = schedule.latest(first.step);
            self.advance_to_deadline(deadline, schedule);
            let mut compatible = self.compatible_candidates(first, reuse);
            self.consume(first);
            if compatible.len() < 2 {
                continue;
            }

            let proposed_wave = earliest_shared(&compatible);
            // a call another one could copy from mustn't move to or past where that one runs
            compatible.retain(|call| {
                earliest(call) >= proposed_wave
                    || !self.has_copier_between(call, earliest(call), proposed_wave, reuse)
            });
            for &call in &compatible {
                self.consume(call);
            }
            if compatible.len() < 2 {
                continue;
            }
            let target_wave = earliest_shared(&compatible);
            // nothing to gain when they'd all run in that wave anyway
            if compatible.iter().all(|call| earliest(call) == target_wave) {
                continue;
            }
            return Some(BatchMove {
                steps: compatible.iter().map(|call| call.step).collect(),
                wave: target_wave,
            });
        }
        None
    }

    /// The call left with the earliest deadline.
    fn next_by_deadline(&mut self) -> Option<Candidate> {
        while let Some(&call) = self.by_latest.last() {
            if !self.consumed[call.subgraph_order] {
                return Some(call);
            }
            self.by_latest.pop();
        }
        None
    }

    /// Makes ready the calls that can run by `wave`.
    fn advance_to_deadline(&mut self, wave: usize, schedule: &Schedule) {
        while let Some(&call) = self.not_ready.last() {
            if schedule.earliest(call.step) > wave {
                break;
            }
            self.not_ready.pop();
            if !self.consumed[call.subgraph_order] {
                self.ready_set(call).insert(call);
            }
        }
    }

    /// `first` and the ready calls that can go in one request with it, in plan order. Never a
    /// call and one that could copy from it. Calls of one selection group conflict, so only the
    /// first ready call of each group has a chance.
    fn compatible_candidates(&self, first: Candidate, reuse: &mut ReuseIndex) -> Vec<Candidate> {
        let heads = self
            .ready_by_group
            .values()
            .filter_map(|calls| calls.first());
        let mut candidates: Vec<Candidate> = self.ready.iter().chain(heads).copied().collect();
        candidates.sort_unstable();

        let mut compatible = vec![first];
        let mut joined: Vec<SelectionGroupId> = first.group.into_iter().collect();
        for call in candidates.into_iter().filter(|&call| call != first) {
            if let Some(group) = call.group {
                if joined.iter().any(|&other| reuse.conflict(group, other)) {
                    continue;
                }
                joined.push(group);
            }
            compatible.push(call);
        }
        compatible
    }

    /// Does a call that could copy from `call` run after `from` and by `to`? Moving `call` from
    /// wave `from` to `to` would take its entities away from such a call. One that runs by
    /// `from` couldn't copy from it anyway, and that includes `call` itself.
    fn has_copier_between(
        &self,
        call: &Candidate,
        from: usize,
        to: usize,
        reuse: &mut ReuseIndex,
    ) -> bool {
        let Some(group) = call.group else {
            return false;
        };
        self.group_waves.iter().any(|(&other, waves)| {
            let after_from = waves.partition_point(|&wave| wave <= from);
            waves.get(after_from).is_some_and(|&wave| wave <= to) && reuse.covers(group, other)
        })
    }

    fn consume(&mut self, call: Candidate) {
        self.consumed[call.subgraph_order] = true;
        self.ready_set(call).remove(&call);
    }

    /// Where `call` is kept while it's ready.
    fn ready_set(&mut self, call: Candidate) -> &mut BTreeSet<Candidate> {
        match call.group {
            Some(group) => self.ready_by_group.entry(group).or_default(),
            None => &mut self.ready,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeSet, HashMap};

    use petgraph::graph::NodeIndex;

    use crate::query_planner::{
        ast::{
            merge_path::MergePath,
            selection_item::SelectionItem,
            selection_set::{FieldSelection, SelectionSet},
        },
        planner::fetch::{
            fetch_graph::FetchGraph,
            fetch_step_data::{FetchStepData, FetchStepFlags, FetchStepKind},
            selections::FetchStepSelections,
            state::SingleTypeFetchStep,
        },
        state::supergraph_state::{OperationKind, SubgraphName},
    };

    use super::schedule_entity_batches;

    /// Schedules entity calls and shows the waves they end up in, one line per wave.
    ///
    /// One call per line: `name {fields} after parent, parent`. The subgraph is the name without
    /// its number, so `a1` and `a2` are calls to `a`. Every call is for type `T`, the fields are
    /// what it asks for, used with entity reuse. A call without `after` runs right after the root.
    fn schedule(calls: &str, entity_reuse: bool) -> String {
        let mut graph = FetchGraph::<SingleTypeFetchStep>::new(OperationKind::Query);
        let root = add_step(&mut graph, "root", FetchStepKind::Root, &[]);
        let mut names: Vec<&str> = Vec::new();
        let mut steps: HashMap<&str, NodeIndex> = HashMap::new();
        for line in calls.lines().map(str::trim).filter(|line| !line.is_empty()) {
            let (call, parents) = match line.split_once(" after ") {
                Some((call, parents)) => (call, parents.split(", ").collect()),
                None => (line, Vec::new()),
            };
            let (name, fields) = match call.split_once(" {") {
                Some((name, fields)) => (
                    name,
                    fields.trim_end_matches('}').split_whitespace().collect(),
                ),
                None => (call, Vec::new()),
            };
            let subgraph = name.trim_end_matches(|c: char| c.is_ascii_digit());
            let step = add_step(&mut graph, subgraph, FetchStepKind::Entity, &fields);
            if parents.is_empty() {
                graph.connect(root, step);
            }
            for parent in parents {
                graph.connect(steps[parent], step);
            }
            names.push(name);
            steps.insert(name, step);
        }
        graph.root_index = Some(root);
        let graph = graph.to_multi_type();
        let waves = schedule_entity_batches(&graph, root, entity_reuse).unwrap();

        let mut by_wave: Vec<Vec<&str>> = Vec::new();
        for name in names {
            let wave = waves[steps[name].index()];
            if by_wave.len() <= wave {
                by_wave.resize_with(wave + 1, Vec::new);
            }
            by_wave[wave].push(name);
        }
        by_wave
            .iter()
            .enumerate()
            .map(|(wave, names)| format!("wave {wave}: {}", names.join(" ")))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn add_step(
        graph: &mut FetchGraph<SingleTypeFetchStep>,
        subgraph: &str,
        kind: FetchStepKind,
        fields: &[&str],
    ) -> NodeIndex {
        let mut output = FetchStepSelections::new("T");
        let asked = SelectionSet {
            items: fields
                .iter()
                .map(|&name| {
                    SelectionItem::Field(FieldSelection {
                        name: name.to_string(),
                        ..FieldSelection::new_typename()
                    })
                })
                .collect(),
        };
        output.add(&asked).unwrap();
        let id = graph.create_fetch_id();
        graph.add_step(FetchStepData {
            id,
            service_name: SubgraphName(subgraph.to_string()),
            response_path: MergePath::default(),
            input: FetchStepSelections::new("T"),
            output,
            kind,
            operation_kind: OperationKind::Query,
            flags: FetchStepFlags::empty(),
            condition: None,
            gated_by: Vec::new(),
            variable_usages: None,
            variable_definitions: None,
            mutation_field_position: None,
            input_rewrites: None,
            output_rewrites: None,
            context_rewrites: None,
            context_variable_definitions: Vec::new(),
            context_producers: BTreeSet::new(),
        })
    }

    /// Without moves it's 4 requests: `c` in waves 0 and 1, `a` and `b` in wave 1.
    ///
    /// `c1` could join `c2` in wave 1, one `c` request less. But `a1` and `b1` wait for `c1`, so
    /// they'd go to wave 2, away from the rest of their batches: an `a` and a `b` request more.
    /// 5 requests is worse than 4, so nothing moves.
    #[test]
    fn falls_back_when_batch_cost_increases() {
        let calls = "
            c1
            x
            a1 after c1
            a2 after x
            a3 after x
            b1 after c1
            b2 after x
            b3 after x
            c2 after x
            d after a2
        ";
        insta::assert_snapshot!(schedule(calls, false), @r"
        wave 0: c1 x
        wave 1: a1 a2 a3 b1 b2 b3 c2
        wave 2: d
        ");
    }

    /// The same, but `c` has the most calls, so it's looked at before `a` and `b`. And `a2` and
    /// `b2` have to stay in wave 1, `d1` and `d2` wait for them, so `a1` and `b1` can't get their
    /// batches back in wave 2 either. Still more requests than without moves, so nothing moves.
    #[test]
    fn falls_back_when_batch_cost_increases_and_its_subgraph_comes_first() {
        let calls = "
            c1
            x
            a1 after c1
            a2 after x
            a3 after x
            b1 after c1
            b2 after x
            b3 after x
            c2 after x
            c3 after x
            c4 after x
            d1 after a2
            d2 after b2
        ";
        insta::assert_snapshot!(schedule(calls, false), @r"
        wave 0: c1 x
        wave 1: a1 a2 a3 b1 b2 b3 c2 c3 c4
        wave 2: d1 d2
        ");
    }

    /// Without moves it's 3 requests: `c` in waves 0 and 1, `a` in wave 1.
    ///
    /// `c1` could join `c2` in wave 1, one `c` request less. But `a1` waits for `c1`, so it'd go
    /// to wave 2, away from `a2` and `a3`: an `a` request more. Still 3 requests, and `c1` and
    /// `a1` would start later, so nothing moves.
    #[test]
    fn keeps_the_waves_when_moves_save_no_request() {
        let calls = "
            c1
            x
            a1 after c1
            a2 after x
            a3 after x
            c2 after x
            d after a2
        ";
        insta::assert_snapshot!(schedule(calls, false), @r"
        wave 0: c1 x
        wave 1: a1 a2 a3 c2
        wave 2: d
        ");
    }

    /// With entity reuse on, `s3` could copy from `s2`, it asks for less. So the two can't share
    /// a request. Both could join `s1` in wave 2, `s3` does: that empties wave 0 of `s` calls,
    /// and in wave 2 it runs after `s2`, so it can copy from it.
    #[test]
    fn of_two_calls_that_could_copy_from_each_other_one_joins() {
        let calls = "
            x1
            x2 after x1
            s1 {c} after x2
            y
            s2 {a b} after y
            s3 {a}
        ";
        insta::assert_snapshot!(schedule(calls, true), @r"
        wave 0: x1 y
        wave 1: x2 s2
        wave 2: s1 s3
        ");
    }

    /// With entity reuse on, `s2` could copy from `s3`, but it runs before `s3`, so it never does.
    /// That doesn't stop `s3` from joining `s1` in wave 2. `s2` itself can't move: `d` and `e`
    /// wait for it, in waves 1 and 2.
    #[test]
    fn a_call_that_runs_before_its_source_doesnt_hold_it() {
        let calls = "
            x1
            x2 after x1
            s1 {z} after x2
            s2 {a}
            d after s2
            e after d
            y
            s3 {a b} after y
        ";
        insta::assert_snapshot!(schedule(calls, true), @r"
        wave 0: x1 s2 y
        wave 1: x2 d
        wave 2: s1 e s3
        ");
    }

    /// With entity reuse on, calls asking for the same thing could copy from each other, so none
    /// of them join. Each round places one call, and the rest of its group is skipped as a
    /// whole, not looked at one by one: that'd be quadratic.
    #[test]
    fn many_calls_asking_for_the_same_thing() {
        let numbers = 1..=3000;
        let ps: Vec<String> = numbers.clone().map(|i| format!("p{i}")).collect();
        let ss: Vec<String> = numbers.map(|i| format!("s{i}")).collect();
        let calls: String = ps
            .iter()
            .zip(&ss)
            .map(|(p, s)| format!("{p}\n{s} after {p}\n"))
            .collect();
        let waves = format!("wave 0: {}\nwave 1: {}", ps.join(" "), ss.join(" "));
        assert_eq!(schedule(&calls, true), waves);
    }
}
