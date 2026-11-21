use crate::query_planner::ast::arguments::ArgumentsMap;
use crate::query_planner::ast::merge_path::{Condition, FieldPathSegment, MergePath, Segment};
use crate::query_planner::ast::operation::{OperationDefinition, VariableDefinition};
use crate::query_planner::ast::selection_item::SelectionItem;
use crate::query_planner::ast::selection_set::{
    response_keys_conflict, FieldSelection, InlineFragmentSelection, SelectionSet,
};
use crate::query_planner::ast::type_aware_selection::TypeAwareSelection;
use crate::query_planner::ast::value::Value;
use crate::query_planner::graph::edge::{
    Edge, EdgeReference, FieldMove, InterfaceObjectTypeMove, PlannerOverrideContext, ReentryMove,
};
use crate::query_planner::graph::node::Node;
use crate::query_planner::graph::Graph;
use crate::query_planner::planner::fetch::fetch_step_data::{
    FetchStepData, FetchStepFlags, FetchStepKind,
};
use crate::query_planner::planner::fetch::response_keys::ResponseKeys;
use crate::query_planner::planner::fetch::selections::FetchStepSelections;
use crate::query_planner::planner::fetch::state::{MultiTypeFetchStep, SingleTypeFetchStep};
use crate::query_planner::planner::plan_nodes::FetchId;
use crate::query_planner::planner::plan_nodes::{
    FetchNodePathSegment, FetchRewrite, KeyRenamer, ValueSetter,
};
use crate::query_planner::planner::tree::query_tree::QueryTree;
use crate::query_planner::planner::tree::query_tree_node::{MutationFieldPosition, QueryTreeNode};
use crate::query_planner::planner::walker::path::OperationPath;
use crate::query_planner::planner::walker::pathfinder::can_satisfy_edge;
use crate::query_planner::planner::QueryPlannerOptions;
use crate::query_planner::state::supergraph_state::{OperationKind, SubgraphName, SupergraphState};
use crate::query_planner::utils::cancellation::CancellationToken;
use ahash::{HashMap, HashMapExt};
use petgraph::algo::has_path_connecting;
use petgraph::stable_graph::{EdgeIndex, NodeIndex, NodeIndices, NodeReferences, StableDiGraph};
use petgraph::visit::EdgeRef;
use petgraph::visit::{Bfs, IntoNodeReferences, NodeIndexable};
use petgraph::Directed;
use petgraph::Direction;
use std::collections::{BTreeSet, HashSet, VecDeque};
use std::fmt::{Debug, Display};
use std::rc::Rc;
use tracing::{instrument, trace};

use super::error::FetchGraphError;

/// Where a contextual argument reads its value from. Two arguments with the same one read the
/// same value, so they share a variable.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct ContextVariableKey {
    /// The response path of the ancestor that sets the context
    ancestor_path: MergePath,
    /// The contextual field
    edge_index: EdgeIndex,
    /// Which of the field's contextual arguments
    argument: usize,
    /// Which of the argument's selections, one per type that sets the context
    selection: usize,
    /// How many fields up the ancestor is
    levels: usize,
}

/// What makes two `@requires` fields the same value: the field (and so the subgraph), where it is,
/// the key it's written under, its arguments, and every condition it's under. Whether the client
/// asked for it or another `@requires` did doesn't matter: the step that resolves it is the same
/// kind of step, and the value lands under the same key.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct PlannedRequires {
    edge: EdgeIndex,
    response_path: MergePath,
    response_key: Option<String>,
    arguments: Option<ArgumentsMap>,
    conditions: Vec<Condition>,
    current_condition: Option<Condition>,
}

#[derive(Debug, Clone)]
pub struct FetchGraph<State> {
    pub(crate) graph: StableDiGraph<FetchStepData<State>, ()>,
    pub root_index: Option<NodeIndex>,
    pub(crate) operation_kind: OperationKind,
    /// The step whose output an entity call gets its representations from, so the step that
    /// holds the object the entity call extends. Only kept while the graph is built: merges
    /// would make the indexes stale, so `to_multi_type` drops it.
    pub(crate) entity_owners: HashMap<NodeIndex, NodeIndex>,
    /// For each entity call step and the type it calls (as a graph node), the type and subgraph
    /// its objects came from, as a graph node. By type, as a step can take several, see
    /// `batched`.
    pub(crate) entity_sources: HashMap<(NodeIndex, NodeIndex), NodeIndex>,
    /// For each entity call step and the type it calls (as a graph node), the entity move it was
    /// made for, so the key it calls the subgraph with.
    pub(crate) entity_moves: HashMap<(NodeIndex, NodeIndex), EdgeIndex>,
    /// Entity calls that take the objects of several types at one spot, see `joins_other_types`.
    /// Only kept while the graph is built.
    pub(crate) batched: HashSet<NodeIndex>,
    /// What planning a node's children did, to do again for another type, see `Replay`. Only
    /// kept while the graph is built.
    replays: HashMap<u64, Vec<Rc<Replay>>>,
    /// The writes into steps that were there before, while children are planned, see `Replay`.
    recordings: Vec<Recording>,
    /// `exact_shape` of the query tree nodes seen so far, by address.
    exact_shapes: HashMap<usize, (u64, bool)>,
    /// The keys of the fields the planner adds itself. Only needed while the graph is built.
    pub(crate) response_keys: ResponseKeys,
    pub(crate) context_producers: HashMap<(MergePath, (EdgeIndex, usize)), Vec<NodeIndex>>,
    /// The variable each contextual argument reads its value into, by where it reads it from.
    /// Only needed while the graph is built.
    pub(crate) context_variables: HashMap<ContextVariableKey, String>,
    pub(crate) context_variable_counter: usize,
    /// Whether any step has `context_producers`. Merges only rename producers when one does,
    /// as going through every step on each merge was most of what a merge cost. Only the
    /// multi-type graph reads it, and `to_multi_type` works it out from the steps. Merges don't
    /// add producers, they only combine or rename them.
    pub(crate) uses_context: bool,
    pub(crate) operation_variables: HashSet<String>,
    /// Each step's children by the entity call they make, see `entity_call_hash`, newest last. So
    /// finding a step to reuse doesn't go through all the children, thousands on a big query.
    /// Only kept while the graph is built: until the merges, a step keeps making the same entity
    /// call and no edge goes away.
    pub(crate) children_by_entity_call: Option<HashMap<(NodeIndex, u64), Vec<NodeIndex>>>,
    /// The step that resolves a `@requires` field, by the field and where it's in the response.
    /// The same field at the same place is the same value however many times something needs
    /// it, so it's planned once. Only kept while the graph is built.
    pub(crate) planned_requires: HashMap<PlannedRequires, NodeIndex>,
    /// Whether an edge's requirements resolve right where a node is, without leaving its
    /// subgraph, see `satisfiable_from`. Only kept while the graph is built.
    satisfiable: HashMap<(EdgeIndex, NodeIndex), bool>,
    /// The id the next step gets. Only goes up: the step count drops when a step is removed, and
    /// an id from it could be one a step still has.
    next_fetch_id: u32,
}

impl FetchGraph<SingleTypeFetchStep> {
    pub fn to_multi_type(self) -> FetchGraph<MultiTypeFetchStep> {
        // moves the steps over, same indices, nothing cloned
        let new_graph = self
            .graph
            .map_owned(|_, step| step.into_multi_type(), |_, _| ());
        let uses_context = new_graph
            .node_weights()
            .any(|step| !step.context_producers.is_empty());

        FetchGraph {
            graph: new_graph,
            root_index: self.root_index,
            operation_kind: self.operation_kind,
            entity_owners: HashMap::new(),
            entity_sources: HashMap::new(),
            entity_moves: HashMap::new(),
            batched: HashSet::new(),
            replays: HashMap::new(),
            recordings: Vec::new(),
            exact_shapes: HashMap::new(),
            response_keys: ResponseKeys::default(),
            context_producers: HashMap::new(),
            context_variables: HashMap::new(),
            context_variable_counter: 0,
            uses_context,
            operation_variables: HashSet::new(),
            children_by_entity_call: None,
            planned_requires: HashMap::new(),
            satisfiable: HashMap::new(),
            next_fetch_id: self.next_fetch_id,
        }
    }

    pub fn new(kind: OperationKind) -> Self {
        Self {
            graph: StableDiGraph::new(),
            root_index: None,
            operation_kind: kind,
            entity_owners: HashMap::new(),
            entity_sources: HashMap::new(),
            entity_moves: HashMap::new(),
            batched: HashSet::new(),
            replays: HashMap::new(),
            recordings: Vec::new(),
            exact_shapes: HashMap::new(),
            response_keys: ResponseKeys::default(),
            context_producers: HashMap::new(),
            context_variables: HashMap::new(),
            context_variable_counter: 0,
            uses_context: false,
            operation_variables: HashSet::new(),
            children_by_entity_call: Some(HashMap::new()),
            planned_requires: HashMap::new(),
            satisfiable: HashMap::new(),
            next_fetch_id: 1,
        }
    }

    /// The children of `parent` that may make this entity call, newest first like `children_of`.
    /// They still need checking: different entity calls can share a hash.
    fn same_entity_calls(
        &self,
        parent: NodeIndex,
        subgraph_name: &SubgraphName,
        input_type_name: &str,
        response_path: &MergePath,
        condition: Option<&Condition>,
    ) -> impl Iterator<Item = NodeIndex> + '_ {
        self.children_making(
            parent,
            entity_call_hash(subgraph_name, input_type_name, response_path, condition),
        )
    }

    /// The children of `parent` that take the objects at `response_path`, whatever their type,
    /// see `joins_other_types`. Newest first, and they still need checking too.
    fn batched_entity_calls(
        &self,
        parent: NodeIndex,
        subgraph_name: &SubgraphName,
        response_path: &MergePath,
        condition: Option<&Condition>,
    ) -> impl Iterator<Item = NodeIndex> + '_ {
        self.children_making(
            parent,
            batched_call_hash(subgraph_name, response_path, condition),
        )
    }

    fn children_making(
        &self,
        parent: NodeIndex,
        entity_call: u64,
    ) -> impl Iterator<Item = NodeIndex> + '_ {
        self.children_by_entity_call
            .as_ref()
            .expect("the children are indexed until the graph is built")
            .get(&(parent, entity_call))
            .into_iter()
            .flat_map(|children| children.iter().rev().copied())
            // nothing updates the index when an edge goes away or a step changes, so catch that
            // in tests instead of quietly reusing the wrong step
            .inspect(move |&child| {
                debug_assert!(
                    self.parents_of(child).any(|edge| edge.source() == parent),
                    "indexed step [{}] is no longer a child of [{}]",
                    child.index(),
                    parent.index(),
                );
                debug_assert_eq!(
                    self.entity_call_hash_of(child),
                    Some(entity_call),
                    "indexed step [{}] changed what it's reused on",
                    child.index(),
                );
            })
    }
}

impl FetchGraph<SingleTypeFetchStep> {
    /// Keeps `write` for every recording that started after `step` was made, see `Replay`.
    fn record(&mut self, step: NodeIndex, write: impl Fn() -> Write) {
        for recording in self.recordings.iter_mut() {
            if step.index() < recording.first_new {
                recording.writes.push(write());
            }
        }
    }

    /// Something changed `step` that a replay can't do again, see `Replay`. The recordings it
    /// was there before can't be replayed.
    fn touch(&mut self, step: NodeIndex) {
        for recording in self.recordings.iter_mut() {
            if step.index() < recording.first_new {
                recording.broken = true;
            }
        }
    }

    /// Adds `selection_set` at `fetch_path` of the output of `step`, for the objects at
    /// `response_path`.
    fn add_output(
        &mut self,
        step: NodeIndex,
        response_path: &MergePath,
        fetch_path: &MergePath,
        selection_set: SelectionSet,
    ) -> Result<(), FetchGraphError> {
        if !self.recordings.is_empty() {
            self.record(step, || Write::Output {
                step,
                response_path: response_path.clone(),
                fetch_path: fetch_path.clone(),
                selection_set: selection_set.clone(),
            });
        }
        self.get_step_data_mut(step)?
            .add_output_at(response_path, fetch_path, selection_set)?;
        Ok(())
    }

    /// Adds `__typename` at `fetch_path` of the output of `step`, for the objects at
    /// `response_path`.
    fn add_output_typename(
        &mut self,
        step: NodeIndex,
        response_path: &MergePath,
        fetch_path: &MergePath,
    ) -> Result<(), FetchGraphError> {
        if !self.recordings.is_empty() {
            self.record(step, || Write::Typename {
                step,
                response_path: response_path.clone(),
                fetch_path: fetch_path.clone(),
            });
        }
        self.get_step_data_mut(step)?
            .add_output_typename_at(response_path, fetch_path)?;
        Ok(())
    }

    /// Adds `selection_set` to the input of `step` for `type_name`.
    fn add_input(
        &mut self,
        step: NodeIndex,
        type_name: &str,
        selection_set: &SelectionSet,
    ) -> Result<(), FetchGraphError> {
        if !self.recordings.is_empty() {
            self.record(step, || Write::Input {
                step,
                type_name: type_name.to_string(),
                selection_set: selection_set.clone(),
            });
        }
        self.get_step_data_mut(step)?
            .input
            .add_to(type_name, selection_set)?;
        Ok(())
    }

    /// A key or requirement of the objects at `response_path`, the way an entity call reads it.
    fn read_at<'selection>(
        &mut self,
        response_path: &MergePath,
        selection: &TypeAwareSelection<'selection>,
    ) -> TypeAwareSelection<'selection> {
        TypeAwareSelection {
            type_name: selection.type_name,
            selection_set: self
                .response_keys
                .input(response_path, &selection.selection_set),
        }
    }

    /// The alias of a field on the objects at `response_path`. The client's, or for a field the
    /// planner adds itself, the key it gets.
    fn alias_at(
        &mut self,
        response_path: &MergePath,
        query_node: &QueryTreeNode,
        field_name: &str,
        created_from_requires: bool,
    ) -> Option<String> {
        if !created_from_requires {
            return query_node.selection_alias().map(|alias| alias.to_string());
        }

        let field = FieldSelection {
            name: field_name.to_string(),
            arguments: query_node.selection_arguments().cloned(),
            ..Default::default()
        };
        let key = self.response_keys.wire_key(response_path, &field);
        (key != field.name).then_some(key)
    }
}

impl<State> FetchGraph<State> {
    /// The hash of the entity call a step in the graph makes, what `children_by_entity_call`
    /// files it under: `entity_call_hash`, or `batched_call_hash` when it takes several types.
    fn entity_call_hash_of(&self, index: NodeIndex) -> Option<u64> {
        let step = self.graph.node_weight(index)?;
        if self.batched.contains(&index) {
            return Some(batched_call_hash(
                &step.service_name,
                &step.response_path,
                step.condition.as_ref(),
            ));
        }
        Some(entity_call_hash(
            &step.service_name,
            step.input.first_definition_name()?,
            &step.response_path,
            step.condition.as_ref(),
        ))
    }

    pub fn all_nodes(&self) -> NodeReferences<'_, FetchStepData<State>> {
        self.graph.node_references()
    }

    pub fn create_fetch_id(&mut self) -> FetchId {
        let id = FetchId(self.next_fetch_id);
        // a plan with more fetches than that doesn't get past the request limits
        self.next_fetch_id = self
            .next_fetch_id
            .checked_add(1)
            .expect("fewer than 2^32 fetches");
        id
    }

    pub fn get_step_data_mut(
        &mut self,
        index: NodeIndex,
    ) -> Result<&mut FetchStepData<State>, FetchGraphError> {
        self.graph.node_weight_mut(index).ok_or_else(|| {
            FetchGraphError::MissingStep(
                index.index(),
                String::from("when getting mutable step data"),
            )
        })
    }
}

impl<State> FetchGraph<State> {
    pub fn parents_of(&self, index: NodeIndex) -> petgraph::stable_graph::Edges<'_, (), Directed> {
        self.graph.edges_directed(index, Direction::Incoming)
    }

    pub fn children_of(&self, index: NodeIndex) -> petgraph::stable_graph::Edges<'_, (), Directed> {
        self.graph.edges_directed(index, Direction::Outgoing)
    }

    pub fn is_descendant_of(&self, descendant: NodeIndex, ancestor: NodeIndex) -> bool {
        has_path_connecting(&self.graph, ancestor, descendant, None)
    }

    /// Checks if one is ancestor of the other and vice versa
    pub fn is_ancestor_or_descendant(&self, a: NodeIndex, b: NodeIndex) -> bool {
        self.is_descendant_of(a, b) || self.is_descendant_of(b, a)
    }

    pub fn step_indices<'a>(&'a self) -> NodeIndices<'a, FetchStepData<State>> {
        self.graph.node_indices()
    }

    pub fn get_step_data(
        &self,
        index: NodeIndex,
    ) -> Result<&FetchStepData<State>, FetchGraphError> {
        self.graph.node_weight(index).ok_or_else(|| {
            FetchGraphError::MissingStep(index.index(), String::from("when getting step data"))
        })
    }

    pub fn connect(&mut self, parent_index: NodeIndex, child_index: NodeIndex) -> EdgeIndex {
        // from the child's side, as a step has a parent or two but can have thousands of children
        if let Some(edge) = self
            .parents_of(child_index)
            .find(|edge| edge.source() == parent_index)
        {
            return edge.id();
        }
        let edge = self.graph.add_edge(parent_index, child_index, ());
        // only while the graph is built, see `children_by_entity_call`
        let entity_call = self
            .children_by_entity_call
            .as_ref()
            .and_then(|_| self.entity_call_hash_of(child_index));
        if let (Some(index), Some(entity_call)) =
            (self.children_by_entity_call.as_mut(), entity_call)
        {
            index
                .entry((parent_index, entity_call))
                .or_default()
                .push(child_index);
        }
        edge
    }

    pub fn remove_edge(&mut self, edge_index: EdgeIndex) -> bool {
        self.graph.remove_edge(edge_index).is_some_and(|_| true)
    }

    #[instrument(level = "trace", skip_all, fields(
      index = index.index(),
    ))]
    pub fn remove_step(&mut self, index: NodeIndex) -> bool {
        self.graph.remove_node(index).is_some_and(|_| true)
    }

    pub fn add_step(&mut self, data: FetchStepData<State>) -> NodeIndex {
        self.uses_context |= !data.context_producers.is_empty();
        self.graph.add_node(data)
    }

    pub fn bfs<F>(&self, root_index: NodeIndex, mut visitor: F) -> Option<NodeIndex>
    where
        F: FnMut(&NodeIndex, &FetchStepData<State>) -> bool,
    {
        self.graph.node_weight(root_index)?;

        let mut bfs = Bfs::new(&self.graph, root_index);

        while let Some(step_index) = bfs.next(&self.graph) {
            // Get the data for the current step. bfs.next() should yield valid indices.
            let step_data = self
                .graph
                .node_weight(step_index)
                .expect("BFS returned invalid node index");

            if visitor(&step_index, step_data) {
                return Some(step_index);
            }
        }

        None
    }

    /// Whether a field or fragment that `step_index` gets under `condition` can leave the
    /// condition out, because a step that has it is skipped when it's false: the step itself, or
    /// the step the field is fetched for (a key or a `@requires` field)
    pub fn is_condition_on_step(
        &self,
        step_index: NodeIndex,
        requiring_step_index: Option<NodeIndex>,
        condition: &Condition,
    ) -> bool {
        let has_condition = |index: NodeIndex| {
            self.get_step_data(index)
                .is_ok_and(|step| step.condition.as_ref() == Some(condition))
        };
        has_condition(step_index) || requiring_step_index.is_some_and(has_condition)
    }

    /// The condition a field or fragment that `step_index` gets at `fetch_path` has to carry.
    /// `None` when something already applies it: a field or fragment on the way, like
    /// `... | [Product] @include(if: $bool)`, or a step, see `is_condition_on_step`.
    pub fn condition_to_apply(
        &self,
        step_index: NodeIndex,
        requiring_step_index: Option<NodeIndex>,
        fetch_path: &MergePath,
        condition: Option<&Condition>,
    ) -> Option<Condition> {
        let condition = condition?;
        let condition_in_path = fetch_path.inner.iter().any(|segment| {
            matches!(
                segment,
                Segment::TypeCondition(_, Some(c)) | Segment::Field(_, _, Some(c)) if c == condition
            )
        });
        if condition_in_path
            || self.is_condition_on_step(step_index, requiring_step_index, condition)
        {
            return None;
        }
        Some(condition.clone())
    }
}

impl FetchGraph<MultiTypeFetchStep> {
    #[instrument(level = "trace", skip_all)]
    pub fn collect_variable_usages(&mut self) -> Result<(), FetchGraphError> {
        let nodes_idx = self.graph.node_indices().collect::<Vec<_>>();

        for node_idx in nodes_idx {
            let step_data = self.get_step_data_mut(node_idx)?;
            let usage = step_data.output.variable_usages();

            if !usage.is_empty() {
                step_data.variable_usages = Some(usage);
            }
        }

        Ok(())
    }

    pub fn get_pair_of_steps_mut(
        &mut self,
        index1: NodeIndex,
        index2: NodeIndex,
    ) -> Result<
        (
            &mut FetchStepData<MultiTypeFetchStep>,
            &mut FetchStepData<MultiTypeFetchStep>,
        ),
        FetchGraphError,
    > {
        // `index_twice_mut` panics when indexes are equal
        if index1 == index2 {
            return Err(FetchGraphError::SameNodeIndex(index1.index()));
        }

        // `index_twice_mut` panics when nodes do not exist
        if self.graph.node_weight(index1).is_none() {
            return Err(FetchGraphError::MissingStep(
                index1.index(),
                String::from("when checking existence"),
            ));
        }
        if self.graph.node_weight(index2).is_none() {
            return Err(FetchGraphError::MissingStep(
                index2.index(),
                String::from("when checking existence"),
            ));
        }

        Ok(self.graph.index_twice_mut(index1, index2))
    }
}

impl<State> Display for FetchGraph<State> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Nodes:")?;
        for node_index in self.graph.node_indices() {
            // ignore root node
            if node_index.index() == 0 {
                continue;
            }
            if let Some(fetch_step) = self.graph.node_weight(node_index) {
                fetch_step.pretty_write(f, node_index)?;
                writeln!(f)?;
            }
        }

        writeln!(f, "\nTree:")?;

        let mut stack: VecDeque<(NodeIndex, usize)> = VecDeque::new();

        let roots = find_graph_roots(self);

        if roots.is_empty() {
            writeln!(f, "Fetch step graph is empty or has no roots.")?;
            return Ok(());
        }

        if roots.len() > 1 {
            writeln!(f, "Fetch step graph has multiple roots:")?;
            return Ok(());
        }

        for root_index in roots {
            for child_index in self
                .graph
                .edges_directed(root_index, Direction::Outgoing)
                .map(|edge_ref| edge_ref.target())
            {
                stack.push_front((child_index, 0));
            }
        }

        while let Some((node_index, depth)) = stack.pop_back() {
            let indent = "  ".repeat(depth);
            writeln!(f, "{indent}[{}]", node_index.index())?;

            for child_index in self
                .graph
                .edges_directed(node_index, Direction::Outgoing)
                .map(|edge_ref| edge_ref.target())
            {
                stack.push_back((child_index, depth + 1));
            }
        }

        Ok(())
    }
}

/// The `@include`/`@skip` conditions a query node sits under.
#[derive(Clone, Copy, Default)]
struct Conditions<'a> {
    /// All of them, top first. A step created here copies them into `gated_by`.
    from_root: &'a [Condition],
    /// The one the next field or fragment puts on itself as a directive. Usually the last of
    /// `from_root`, but `None` right after entering a subgraph: the field we entered through
    /// already has it.
    current: Option<&'a Condition>,
}

impl Conditions<'_> {
    /// For what comes right after entering a subgraph: the field we entered through already has
    /// `current`, so nothing below puts it on again. `from_root` stays, the fetches still need it.
    fn without_current(self) -> Self {
        Self {
            current: None,
            ..self
        }
    }
}

fn create_noop_fetch_step(
    fetch_graph: &mut FetchGraph<SingleTypeFetchStep>,
    created_from_requires: bool,
) -> NodeIndex {
    let flags = if created_from_requires {
        FetchStepFlags::USED_FOR_REQUIRES
    } else {
        FetchStepFlags::empty()
    };

    let id = fetch_graph.create_fetch_id();
    fetch_graph.add_step(FetchStepData {
        id,
        service_name: SubgraphName::any(),
        response_path: MergePath::default(),
        input: FetchStepSelections::new_empty(),
        output: FetchStepSelections::new_empty(),
        flags,
        condition: None,
        gated_by: Vec::new(),
        kind: FetchStepKind::Root,
        operation_kind: OperationKind::Query,
        input_rewrites: None,
        output_rewrites: None,
        variable_usages: None,
        variable_definitions: None,
        mutation_field_position: None,
        context_rewrites: None,
        context_variable_definitions: Vec::new(),
        context_producers: BTreeSet::new(),
    })
}

#[allow(clippy::too_many_arguments)]
fn create_fetch_step_for_entity_call(
    fetch_graph: &mut FetchGraph<SingleTypeFetchStep>,
    owner_index: NodeIndex,
    subgraph_name: &SubgraphName,
    input_type_name: &str,
    output_type_name: &str,
    response_path: &MergePath,
    conditions: Conditions<'_>,
    used_for_requires: bool,
) -> NodeIndex {
    let condition = conditions.current;
    let flags = if used_for_requires {
        FetchStepFlags::USED_FOR_REQUIRES
    } else {
        FetchStepFlags::empty()
    };
    let mut input = FetchStepSelections::new(input_type_name);
    input
        .add(&SelectionSet {
            items: vec![SelectionItem::Field(FieldSelection::new_typename())],
        })
        .unwrap();

    let id = fetch_graph.create_fetch_id();
    let step_index = fetch_graph.add_step(FetchStepData {
        id,
        service_name: subgraph_name.clone(),
        response_path: response_path.clone(),
        input,
        output: FetchStepSelections::new(output_type_name),
        flags,
        condition: condition.cloned(),
        gated_by: conditions.from_root.to_vec(),
        kind: FetchStepKind::Entity,
        operation_kind: OperationKind::Query,
        input_rewrites: None,
        output_rewrites: None,
        variable_usages: None,
        variable_definitions: None,
        mutation_field_position: None,
        context_rewrites: None,
        context_variable_definitions: Vec::new(),
        context_producers: BTreeSet::new(),
    });
    fetch_graph.entity_owners.insert(step_index, owner_index);

    step_index
}

// TODO: simplfy args
#[allow(clippy::too_many_arguments)]
fn create_fetch_step_for_root_move(
    fetch_graph: &mut FetchGraph<SingleTypeFetchStep>,
    root_step_index: NodeIndex,
    subgraph_name: &SubgraphName,
    type_name: &str,
    operation_kind: OperationKind,
    mutation_field_position: MutationFieldPosition,
    response_path: &MergePath,
    conditions: Conditions<'_>,
) -> NodeIndex {
    let condition = conditions.current;
    let id = fetch_graph.create_fetch_id();
    let idx = fetch_graph.add_step(FetchStepData {
        id,
        service_name: subgraph_name.clone(),
        response_path: response_path.clone(),
        input: FetchStepSelections::new(type_name),
        output: FetchStepSelections::new(type_name),
        flags: FetchStepFlags::empty(),
        condition: condition.cloned(),
        gated_by: conditions.from_root.to_vec(),
        kind: FetchStepKind::Root,
        operation_kind,
        variable_usages: None,
        variable_definitions: None,
        input_rewrites: None,
        output_rewrites: None,
        mutation_field_position,
        context_rewrites: None,
        context_variable_definitions: Vec::new(),
        context_producers: BTreeSet::new(),
    });

    fetch_graph.connect(root_step_index, idx);

    idx
}

/// The entity call a step makes, the part a step gets reused on: its subgraph, input type,
/// response path and condition. Without the types of the type conditions on the path: a step
/// can take more of those later, see `Replay`, and it's still found. The steps found still get
/// their path compared.
fn entity_call_hash(
    subgraph_name: &SubgraphName,
    input_type_name: &str,
    response_path: &MergePath,
    condition: Option<&Condition>,
) -> u64 {
    use std::hash::{Hash, Hasher};

    let mut hasher = ahash::AHasher::default();
    subgraph_name.hash(&mut hasher);
    input_type_name.hash(&mut hasher);
    condition.hash(&mut hasher);
    hash_without_types(&response_path.inner, &mut hasher);
    hasher.finish()
}

/// Planning a node's children again for another type, at the same spot. Type expansion makes
/// `product { ... on Book { reviews { ... } } ... on Movie { reviews { ... } } }`, and below
/// `reviews` it's the same moves for every type, the same subgraphs and fields. Planned for
/// `Book`, it made steps at `product|[Book].reviews.@...` and wrote into steps that were there
/// before, like the one `product` comes from. For `Movie`, the steps it made take `Movie` at that
/// type condition too, `product|[Book|Movie].reviews.@`, and the writes into the steps that were
/// there are done again, for `Movie`. The same steps and the same fields as planning it again
/// and batching the steps after.
///
/// Only when the step the children are planned in is an entity call that takes both types at
/// the end of its path, see `joins_other_types`, so what's written into it lands with the right
/// type. A replay that wouldn't come out the same, see `Replay::apply`, plans the children again.
#[derive(Debug, Clone)]
struct Replay {
    /// Where the children were planned.
    response_path: MergePath,
    /// The type condition the types differ at.
    slot: usize,
    /// The steps made while planning them, by index.
    made: std::ops::Range<usize>,
    /// The writes into steps that were there before.
    writes: Vec<Write>,
    /// What planning them returned.
    leaves: Vec<NodeIndex>,
}

/// A write into a step, kept by a recording, see `Replay`.
#[derive(Debug, Clone)]
enum Write {
    Output {
        step: NodeIndex,
        response_path: MergePath,
        fetch_path: MergePath,
        selection_set: SelectionSet,
    },
    Typename {
        step: NodeIndex,
        response_path: MergePath,
        fetch_path: MergePath,
    },
    Input {
        step: NodeIndex,
        type_name: String,
        selection_set: SelectionSet,
    },
}

/// The writes into steps made before `first_new`, while children are planned, see `Replay`.
#[derive(Debug, Clone)]
struct Recording {
    first_new: usize,
    writes: Vec<Write>,
    /// A step that was there got changed in a way a replay can't repeat.
    broken: bool,
}

/// `path`, with `type_name` at the type condition at `slot`.
fn with_type_at(path: &MergePath, slot: usize, type_name: &str) -> MergePath {
    MergePath::new(
        path.inner
            .iter()
            .enumerate()
            .map(|(position, segment)| match segment {
                Segment::TypeCondition(_, condition) if position == slot => Segment::TypeCondition(
                    Rc::new(BTreeSet::from([type_name.to_string()])),
                    condition.clone(),
                ),
                segment => segment.clone(),
            })
            .collect(),
    )
}

impl Replay {
    /// Does it again for `type_name`. `None`, having changed nothing, when it wouldn't come out
    /// like planning the children again: a step made then isn't at that type condition, or a
    /// write goes into a step below it that doesn't take `type_name`.
    fn apply(
        &self,
        fetch_graph: &mut FetchGraph<SingleTypeFetchStep>,
        type_name: &str,
    ) -> Result<Option<Vec<NodeIndex>>, FetchGraphError> {
        let slot = self.slot;
        let at_slot = |path: &MergePath| match path.inner.get(slot) {
            Some(Segment::TypeCondition(type_names, _)) => Some(type_names.clone()),
            _ => None,
        };
        for index in self.made.clone() {
            let step = fetch_graph.get_step_data(NodeIndex::new(index))?;
            if at_slot(&step.response_path).is_none() {
                return Ok(None);
            }
        }
        for write in &self.writes {
            let step = match write {
                Write::Output { step, .. } | Write::Typename { step, .. } => *step,
                // what an entity call reads doesn't depend on where it is
                Write::Input { .. } => continue,
            };
            let path = &fetch_graph.get_step_data(step)?.response_path;
            if path.len() > slot
                && !at_slot(path).is_some_and(|type_names| type_names.contains(type_name))
            {
                return Ok(None);
            }
        }

        for index in self.made.clone() {
            let step = fetch_graph.get_step_data_mut(NodeIndex::new(index))?;
            if let Some(Segment::TypeCondition(type_names, condition)) =
                step.response_path.inner.get(slot)
            {
                if !type_names.contains(type_name) {
                    let mut type_names = BTreeSet::clone(type_names);
                    type_names.insert(type_name.to_string());
                    let segment = Segment::TypeCondition(Rc::new(type_names), condition.clone());
                    step.response_path = MergePath::new(
                        step.response_path
                            .inner
                            .iter()
                            .enumerate()
                            .map(|(position, old)| {
                                if position == slot {
                                    segment.clone()
                                } else {
                                    old.clone()
                                }
                            })
                            .collect(),
                    );
                }
            }
        }
        for write in &self.writes {
            match write {
                Write::Output {
                    step,
                    response_path,
                    fetch_path,
                    selection_set,
                } => {
                    let fetch_path =
                        fetch_path_for(fetch_graph, *step, fetch_path, slot, type_name)?;
                    fetch_graph.add_output(
                        *step,
                        &with_type_at(response_path, slot, type_name),
                        &fetch_path,
                        selection_set.clone(),
                    )?;
                }
                Write::Typename {
                    step,
                    response_path,
                    fetch_path,
                } => {
                    let fetch_path =
                        fetch_path_for(fetch_graph, *step, fetch_path, slot, type_name)?;
                    fetch_graph.add_output_typename(
                        *step,
                        &with_type_at(response_path, slot, type_name),
                        &fetch_path,
                    )?;
                }
                Write::Input {
                    step,
                    type_name: input_type,
                    selection_set,
                } => fetch_graph.add_input(*step, input_type, selection_set)?,
            }
        }
        Ok(Some(self.leaves.clone()))
    }
}

/// `fetch_path` of a write into `step`, for `type_name` at the type condition at `slot` of the
/// response path. In a step above that type condition it's on the fetch path too.
fn fetch_path_for(
    fetch_graph: &FetchGraph<SingleTypeFetchStep>,
    step: NodeIndex,
    fetch_path: &MergePath,
    slot: usize,
    type_name: &str,
) -> Result<MergePath, FetchGraphError> {
    let depth = fetch_graph.get_step_data(step)?.response_path.len();
    Ok(if depth <= slot {
        with_type_at(fetch_path, slot - depth, type_name)
    } else {
        fetch_path.clone()
    })
}

/// A hash of what's planned below `node`, exactly: the edges, aliases, arguments and conditions,
/// and the requirements, see `Replay`. And whether there's an entity call or a requirement below
/// it: only planning those costs more than recording what it writes.
fn exact_shape(
    graph: &Graph<'_>,
    shapes: &mut HashMap<usize, (u64, bool)>,
    node: &QueryTreeNode,
) -> (u64, bool) {
    use std::hash::{Hash, Hasher};

    let address = node as *const QueryTreeNode as usize;
    if let Some(&shape) = shapes.get(&address) {
        return shape;
    }
    let mut hasher = ahash::AHasher::default();
    node.edge_from_parent.hash(&mut hasher);
    node.selection_alias().hash(&mut hasher);
    node.selection_arguments()
        .map(|arguments| arguments.hash_u64())
        .hash(&mut hasher);
    node.condition.hash(&mut hasher);
    node.context.is_some().hash(&mut hasher);
    node.mutation_field_position.hash(&mut hasher);
    let mut heavy = !node.requirements.is_empty()
        || node.edge_from_parent.is_some_and(|edge| {
            !matches!(
                graph.edge(edge),
                Ok(Edge::FieldMove(_) | Edge::AbstractMove(_) | Edge::Selfie(_))
            )
        });
    for (tag, nodes) in [(0u8, &node.requirements), (1, &node.children)] {
        (tag, nodes.len()).hash(&mut hasher);
        for child in nodes {
            let (shape, child_heavy) = exact_shape(graph, shapes, child);
            shape.hash(&mut hasher);
            heavy |= child_heavy;
        }
    }
    let shape = (hasher.finish(), heavy);
    shapes.insert(address, shape);
    shape
}

/// `exact_shape` of the children of `node`, together.
fn children_shape(
    graph: &Graph<'_>,
    shapes: &mut HashMap<usize, (u64, bool)>,
    node: &QueryTreeNode,
) -> (u64, bool) {
    use std::hash::{Hash, Hasher};

    let mut hasher = ahash::AHasher::default();
    let mut heavy = false;
    node.children.len().hash(&mut hasher);
    for child in node.children.iter() {
        let (shape, child_heavy) = exact_shape(graph, shapes, child);
        shape.hash(&mut hasher);
        heavy |= child_heavy;
    }
    (hasher.finish(), heavy)
}

/// Hashes `segments` without the types of their type conditions.
fn hash_without_types(segments: &[Segment], hasher: &mut ahash::AHasher) {
    use std::hash::Hash;

    segments.len().hash(hasher);
    for segment in segments {
        match segment {
            Segment::TypeCondition(_, condition) => {
                0u8.hash(hasher);
                condition.hash(hasher);
            }
            segment => {
                1u8.hash(hasher);
                segment.hash(hasher);
            }
        }
    }
}

/// Like `entity_call_hash`, for an entity call that takes the objects of several types at the
/// end of `response_path`, see `joins_other_types`. Every type there gets the same hash.
fn batched_call_hash(
    subgraph_name: &SubgraphName,
    response_path: &MergePath,
    condition: Option<&Condition>,
) -> u64 {
    use std::hash::{Hash, Hasher};

    let mut hasher = ahash::AHasher::default();
    subgraph_name.hash(&mut hasher);
    condition.hash(&mut hasher);
    if let Some((Segment::TypeCondition(_, type_condition), rest)) =
        response_path.inner.split_last()
    {
        hash_without_types(rest, &mut hasher);
        type_condition.hash(&mut hasher);
    }
    // so it's never the hash of a plain call to the same spot
    "batched".hash(&mut hasher);
    hasher.finish()
}

/// Whether an entity call for `input_type_name` at `response_path` can take the objects of
/// other types there too, as one call: the path ends with a type condition on that type, like
/// `search.@|[Book]`, and the call reads and writes that type. The calls for `Movie` and `Song`
/// there go to the same subgraph with their own key and fields, and each type gets its own
/// selection set, so they can't get in each other's way.
fn joins_other_types(
    response_path: &MergePath,
    input_type_name: &str,
    output_type_name: &str,
) -> bool {
    if input_type_name != output_type_name {
        return false;
    }
    let Some((Segment::TypeCondition(type_names, _), rest)) = response_path.inner.split_last()
    else {
        return false;
    };
    type_names.len() == 1
        && type_names.contains(input_type_name)
        // two type conditions in a row mean both, a union of them would be wrong
        && !matches!(rest.last(), Some(Segment::TypeCondition(..)))
}

/// Whether two paths are the same but for the types of the type condition at their end.
fn same_but_last_types(left: &MergePath, right: &MergePath) -> bool {
    match (left.inner.split_last(), right.inner.split_last()) {
        (
            Some((Segment::TypeCondition(_, left_condition), left_rest)),
            Some((Segment::TypeCondition(_, right_condition), right_rest)),
        ) => left_condition == right_condition && left_rest == right_rest,
        _ => false,
    }
}

// TODO: simplfy args
#[allow(clippy::too_many_arguments)]
fn ensure_fetch_step_for_subgraph(
    fetch_graph: &mut FetchGraph<SingleTypeFetchStep>,
    parent_fetch_step_index: NodeIndex,
    subgraph_name: &SubgraphName,
    input_type_name: &str,
    output_type_name: &str,
    response_path: &MergePath,
    key: Option<&TypeAwareSelection<'_>>,
    conditions: Conditions<'_>,
    created_from_requires: bool,
    batchable: bool,
) -> Result<NodeIndex, FetchGraphError> {
    let condition = conditions.current;
    let batched = batchable && joins_other_types(response_path, input_type_name, output_type_name);
    if batched {
        return ensure_batched_fetch_step(
            fetch_graph,
            parent_fetch_step_index,
            subgraph_name,
            input_type_name,
            response_path,
            key,
            conditions,
            created_from_requires,
        );
    }
    let matching_child_index = fetch_graph
        .same_entity_calls(
            parent_fetch_step_index,
            subgraph_name,
            input_type_name,
            response_path,
            condition,
        )
        .find(|&child_index| {
            fetch_graph.get_step_data(child_index).is_ok_and(|step| {
                step.matches_entity_call(subgraph_name, input_type_name, response_path, condition)
                    // the requested key fields have to be part of the input
                    && key.is_none_or(|key| step.input.contains(key.type_name, &key.selection_set))
                    // a step made for requirements is only reused for requirements, and
                    // the other way around: the optimizations keep the two apart too
                    && step.flags.contains(FetchStepFlags::USED_FOR_REQUIRES)
                        == created_from_requires
            })
        });

    match matching_child_index {
        Some(idx) => {
            trace!(
                "found existing fetch step [{}] for entity move key({}) in children of {}",
                idx.index(),
                key.map(|r| r.to_string()).unwrap_or_default(),
                parent_fetch_step_index.index(),
            );
            let step = fetch_graph
                .graph
                .node_weight_mut(idx)
                .ok_or(FetchGraphError::IndexNone)?;
            step.gated_by
                .retain(|condition| conditions.from_root.contains(condition));
            Ok(idx)
        }
        None => {
            let step_index = create_fetch_step_for_entity_call(
                fetch_graph,
                parent_fetch_step_index,
                subgraph_name,
                input_type_name,
                output_type_name,
                response_path,
                conditions,
                created_from_requires,
            );
            if let Some(selection) = key {
                let step = fetch_graph.get_step_data_mut(step_index)?;
                step.input.add(&selection.selection_set)?
            }

            trace!(
                "created a new fetch step [{}] subgraph({}) type({}) key({}) in children of {}",
                step_index.index(),
                subgraph_name,
                input_type_name,
                key.map(|r| r.to_string()).unwrap_or_default(),
                parent_fetch_step_index.index(),
            );

            Ok(step_index)
        }
    }
}

/// The step for an entity call that can take the objects of other types at the same spot too,
/// see `joins_other_types`. One that has the type already, when it was made for the same key,
/// or another key that doesn't clash with it. Otherwise one at that spot that doesn't have the
/// type yet, which takes it on. Or a new one.
#[allow(clippy::too_many_arguments)]
fn ensure_batched_fetch_step(
    fetch_graph: &mut FetchGraph<SingleTypeFetchStep>,
    parent_fetch_step_index: NodeIndex,
    subgraph_name: &SubgraphName,
    type_name: &str,
    response_path: &MergePath,
    key: Option<&TypeAwareSelection<'_>>,
    conditions: Conditions<'_>,
    created_from_requires: bool,
) -> Result<NodeIndex, FetchGraphError> {
    let condition = conditions.current;
    let mut with_type_and_key = None;
    let mut with_type = None;
    let mut without_type = None;
    for child_index in fetch_graph.batched_entity_calls(
        parent_fetch_step_index,
        subgraph_name,
        response_path,
        condition,
    ) {
        let step = fetch_graph.get_step_data(child_index)?;
        if step.service_name != *subgraph_name
            || step.condition.as_ref() != condition
            || step.flags.contains(FetchStepFlags::USED_FOR_REQUIRES) != created_from_requires
            || !same_but_last_types(&step.response_path, response_path)
        {
            continue;
        }
        match step.input.selections_for_definition(type_name) {
            Some(input) => {
                let has_key = key.is_none_or(|key| input.contains(&key.selection_set));
                if has_key {
                    with_type_and_key = Some(child_index);
                    break;
                }
                if with_type.is_none()
                    && key.is_none_or(|key| !response_keys_conflict(input, &key.selection_set))
                {
                    with_type = Some(child_index);
                }
            }
            None => {
                without_type = without_type.or(Some(child_index));
            }
        }
    }

    let step_index = match with_type_and_key.or(with_type).or(without_type) {
        Some(step_index) => step_index,
        None => {
            let step_index = create_fetch_step_for_entity_call(
                fetch_graph,
                parent_fetch_step_index,
                subgraph_name,
                type_name,
                type_name,
                response_path,
                conditions,
                created_from_requires,
            );
            fetch_graph.batched.insert(step_index);
            step_index
        }
    };
    let new_type = fetch_graph
        .get_step_data(step_index)?
        .input
        .selections_for_definition(type_name)
        .is_none();
    if new_type {
        fetch_graph.touch(step_index);
    }
    let step = fetch_graph.get_step_data_mut(step_index)?;
    step.gated_by
        .retain(|condition| conditions.from_root.contains(condition));
    if new_type {
        trace!(
            "fetch step [{}] takes the {} objects at {} too",
            step_index.index(),
            type_name,
            response_path
        );
        step.input.add_definition(type_name);
        step.input.add_to(
            type_name,
            &SelectionSet {
                items: vec![SelectionItem::Field(FieldSelection::new_typename())],
            },
        )?;
        step.output.add_definition(type_name);
        if let Some((Segment::TypeCondition(type_names, type_condition), rest)) =
            step.response_path.inner.split_last()
        {
            let mut type_names = BTreeSet::clone(type_names);
            type_names.insert(type_name.to_string());
            step.response_path = MergePath::new(
                rest.iter()
                    .cloned()
                    .chain(std::iter::once(Segment::TypeCondition(
                        Rc::new(type_names),
                        type_condition.clone(),
                    )))
                    .collect(),
            );
        }
    }
    // the caller adds the key to the input, see `process_entity_move_edge`
    Ok(step_index)
}

fn ensure_fetch_step_for_requirement(
    fetch_graph: &mut FetchGraph<SingleTypeFetchStep>,
    parent_fetch_step_index: NodeIndex,
    subgraph_name: &SubgraphName,
    type_name: &str,
    response_path: &MergePath,
    conditions: Conditions<'_>,
    requirement: &TypeAwareSelection<'_>,
) -> Result<NodeIndex, FetchGraphError> {
    let condition = conditions.current;
    let matching_child_index = fetch_graph
        .same_entity_calls(
            parent_fetch_step_index,
            subgraph_name,
            type_name,
            response_path,
            condition,
        )
        .find(|&child_index| {
            fetch_graph.get_step_data(child_index).is_ok_and(|step| {
                step.matches_entity_call(subgraph_name, type_name, response_path, condition)
                    && step.context_rewrites.is_none()
                    && step
                        .input
                        .contains(requirement.type_name, &requirement.selection_set)
            })
        });

    match matching_child_index {
        Some(idx) => {
            trace!(
                "found existing fetch step [{}] children of {}",
                idx.index(),
                parent_fetch_step_index.index(),
            );
            Ok(idx)
        }
        None => {
            let step_index = create_fetch_step_for_entity_call(
                fetch_graph,
                parent_fetch_step_index,
                subgraph_name,
                type_name,
                type_name,
                response_path,
                conditions,
                true,
            );

            trace!(
                "created a new fetch step [{}] subgraph({}) type({}) requirement({}) in children of {}",
                step_index.index(),
                subgraph_name,
                type_name,
                requirement.to_string(),
                parent_fetch_step_index.index(),
            );

            Ok(step_index)
        }
    }
}

// TODO: simplfy args
#[allow(clippy::too_many_arguments)]
#[instrument(level = "trace", skip_all, fields(
  count = query_node.children.len(),
  parent_fetch_step_index = parent_fetch_step_index.index(),
  requiring_fetch_step_index = requiring_fetch_step_index.map(|s| s.index()),
  fetch_path = fetch_path.to_string(),
  response_path = response_path.to_string()
))]
fn process_children_for_fetch_steps(
    graph: &Graph<'_>,
    fetch_graph: &mut FetchGraph<SingleTypeFetchStep>,
    supergraph: &SupergraphState,
    override_context: &PlannerOverrideContext,
    query_node: &QueryTreeNode,
    parent_fetch_step_index: NodeIndex,
    response_path: &MergePath,
    fetch_path: &MergePath,
    requiring_fetch_step_index: Option<NodeIndex>,
    conditions: Conditions<'_>,
    created_from_requires: bool,
) -> Result<Vec<NodeIndex>, FetchGraphError> {
    if query_node.children.is_empty() {
        return Ok(vec![parent_fetch_step_index]);
    }

    // the same children planned for another type at this spot, see `Replay`
    let replayable = replay_slot(graph, fetch_graph, parent_fetch_step_index, response_path)?;
    // plain fields cost about as much to record as to plan again
    let shape = replayable
        .as_ref()
        .map(|_| children_shape(graph, &mut fetch_graph.exact_shapes, query_node))
        .filter(|(_, heavy)| *heavy);
    let replayable = replayable.filter(|_| shape.is_some());
    let replay_key = replayable.as_ref().map(|(slot, _)| {
        use std::hash::{Hash, Hasher};

        let mut hasher = ahash::AHasher::default();
        parent_fetch_step_index.hash(&mut hasher);
        requiring_fetch_step_index.hash(&mut hasher);
        fetch_path.hash(&mut hasher);
        conditions.from_root.hash(&mut hasher);
        conditions.current.hash(&mut hasher);
        created_from_requires.hash(&mut hasher);
        // the response path is the parent's with the fetch path on it, but for the types at
        // `slot`. It's compared when one is found.
        slot.hash(&mut hasher);
        shape.hash(&mut hasher);
        hasher.finish()
    });
    if let (Some((slot, type_name)), Some(key)) = (&replayable, replay_key) {
        let found = fetch_graph.replays.get(&key).and_then(|replays| {
            replays
                .iter()
                .find(|replay| {
                    replay.slot == *slot
                        && replay.response_path.len() == response_path.len()
                        && replay
                            .response_path
                            .inner
                            .iter()
                            .zip(response_path.inner.iter())
                            .enumerate()
                            .all(|(position, (theirs, ours))| position == *slot || theirs == ours)
                        && replay.response_path.inner[*slot] != response_path.inner[*slot]
                })
                .cloned()
        });
        if let Some(replay) = found {
            if let Some(leaves) = replay.apply(fetch_graph, type_name)? {
                return Ok(leaves);
            }
        }
        fetch_graph.recordings.push(Recording {
            first_new: fetch_graph.graph.node_bound(),
            writes: Vec::new(),
            broken: false,
        });
    }

    let mut planned = || -> Result<Vec<NodeIndex>, FetchGraphError> {
        let mut leaf_fetch_step_indexes: Vec<NodeIndex> = vec![];
        for sub_step in query_node.children.iter() {
            leaf_fetch_step_indexes.extend(process_query_node(
                graph,
                fetch_graph,
                supergraph,
                override_context,
                sub_step,
                Some(parent_fetch_step_index),
                response_path,
                fetch_path,
                requiring_fetch_step_index,
                conditions,
                created_from_requires,
            )?);
        }
        Ok(leaf_fetch_step_indexes)
    };
    let leaves = planned();

    if let (Some((slot, _)), Some(key)) = (replayable, replay_key) {
        let recording = fetch_graph
            .recordings
            .pop()
            .expect("pushed above, and every recording pushed after it was popped");
        if let (Ok(leaves), false) = (&leaves, recording.broken) {
            fetch_graph
                .replays
                .entry(key)
                .or_default()
                .push(Rc::new(Replay {
                    response_path: response_path.clone(),
                    slot,
                    made: recording.first_new..fetch_graph.graph.node_bound(),
                    writes: recording.writes,
                    leaves: leaves.clone(),
                }));
        }
    }
    leaves
}

/// Where children planned in `parent` at `response_path` can be replayed for another type, see
/// `Replay`: the type condition `parent` ends with, when it's an entity call that takes several
/// types there, and the type `response_path` has there.
fn replay_slot(
    graph: &Graph<'_>,
    fetch_graph: &FetchGraph<SingleTypeFetchStep>,
    parent: NodeIndex,
    response_path: &MergePath,
) -> Result<Option<(usize, String)>, FetchGraphError> {
    if graph.has_context_arguments() || !fetch_graph.batched.contains(&parent) {
        return Ok(None);
    }
    let parent_path = &fetch_graph.get_step_data(parent)?.response_path;
    let Some(slot) = parent_path.len().checked_sub(1) else {
        return Ok(None);
    };
    Ok(match response_path.inner.get(slot) {
        Some(Segment::TypeCondition(type_names, _)) if type_names.len() == 1 => type_names
            .first()
            .cloned()
            .map(|type_name| (slot, type_name)),
        _ => None,
    })
}

// TODO: simplfy args
#[allow(clippy::too_many_arguments)]
#[instrument(level = "trace",skip_all, fields(
  count = query_node.requirements.len(),
  fetch_path = fetch_path.to_string(),
  response_path = response_path.to_string()
))]
fn process_requirements_for_fetch_steps(
    graph: &Graph<'_>,
    fetch_graph: &mut FetchGraph<SingleTypeFetchStep>,
    supergraph: &SupergraphState,
    override_context: &PlannerOverrideContext,
    query_node: &QueryTreeNode,
    parent_fetch_step_index: NodeIndex,
    requiring_fetch_step_index: Option<NodeIndex>,
    conditions: Conditions<'_>,
    response_path: &MergePath,
    fetch_path: &MergePath,
) -> Result<(), FetchGraphError> {
    if query_node.requirements.is_empty() {
        return Ok(());
    }

    for req_query_node in query_node.requirements.iter() {
        process_query_node(
            graph,
            fetch_graph,
            supergraph,
            override_context,
            req_query_node,
            Some(parent_fetch_step_index),
            response_path,
            fetch_path,
            requiring_fetch_step_index,
            conditions,
            true,
        )?;
    }

    Ok(())
}

// TODO: simplfy args
#[allow(clippy::too_many_arguments)]
fn process_noop_edge(
    graph: &Graph<'_>,
    fetch_graph: &mut FetchGraph<SingleTypeFetchStep>,
    supergraph: &SupergraphState,
    override_context: &PlannerOverrideContext,
    query_node: &QueryTreeNode,
    parent_fetch_step_index: Option<NodeIndex>,
    response_path: &MergePath,
    fetch_path: &MergePath,
    requiring_fetch_step_index: Option<NodeIndex>,
    conditions: Conditions<'_>,
    created_from_requires: bool,
) -> Result<Vec<NodeIndex>, FetchGraphError> {
    // We're at the root
    let fetch_step_index = parent_fetch_step_index
        .unwrap_or_else(|| create_noop_fetch_step(fetch_graph, created_from_requires));

    process_children_for_fetch_steps(
        graph,
        fetch_graph,
        supergraph,
        override_context,
        query_node,
        fetch_step_index,
        response_path,
        fetch_path,
        requiring_fetch_step_index,
        conditions,
        created_from_requires,
    )
}

/// The object types an entity call asks something of, when it's all in their fragments: every
/// child is `... on Type` or `__typename`. Empty when something is asked of every entity.
fn only_asked_types<'a>(
    graph: &'a Graph<'_>,
    query_node: &QueryTreeNode,
) -> Result<Vec<&'a str>, FetchGraphError> {
    let mut types = Vec::new();
    for child in &query_node.children {
        let Some(edge_index) = child.edge_from_parent else {
            return Ok(Vec::new());
        };
        match graph.edge(edge_index)? {
            Edge::AbstractMove(type_name) => {
                if !types.contains(&type_name.as_str()) {
                    types.push(type_name.as_str());
                }
            }
            Edge::FieldMove(field_move) if field_move.name == "__typename" => {}
            _ => return Ok(Vec::new()),
        }
    }
    Ok(types)
}

// TODO: simplfy args
#[allow(clippy::too_many_arguments)]
#[instrument(level = "trace",skip_all, fields(
  edge = graph.pretty_print_edge(edge_index, false),
  parent_fetch_step_index = parent_fetch_step_index.index(),
  requiring_fetch_step_index = requiring_fetch_step_index.map(|f| f.index()),
))]
fn process_entity_move_edge(
    graph: &Graph<'_>,
    fetch_graph: &mut FetchGraph<SingleTypeFetchStep>,
    supergraph: &SupergraphState,
    override_context: &PlannerOverrideContext,
    query_node: &QueryTreeNode,
    parent_fetch_step_index: NodeIndex,
    response_path: &MergePath,
    fetch_path: &MergePath,
    requiring_fetch_step_index: Option<NodeIndex>,
    edge_index: EdgeIndex,
    conditions: Conditions<'_>,
    created_from_requires: bool,
) -> Result<Vec<NodeIndex>, FetchGraphError> {
    let edge = graph.edge(edge_index)?;
    let (requirement, is_interface) = match edge {
        Edge::EntityMove(em) => (
            TypeAwareSelection {
                selection_set: em.requirements.selection_set.clone(),
                type_name: em.requirements.type_name,
            },
            em.is_interface,
        ),
        _ => {
            return Err(FetchGraphError::UnexpectedEdgeMove(
                "EntityMove".to_string(),
            ))
        }
    };
    let mut requirement = fetch_graph.read_at(response_path, &requirement);

    let head_node_index = graph.get_edge_head(&edge_index)?;
    let head_type = match graph.node(head_node_index)? {
        Node::SubgraphType(t) => t,
        _ => return Err(FetchGraphError::ExpectedSubgraphType),
    };
    let input_type_name = &head_type.name;

    // A key of an interface, like `Animal @key(fields: "id")`, sends every animal. When all that's
    // asked of them is in `... on Dog` and `... on Cat`, a Bird would go with nothing to ask of
    // it, so the input only takes the types that are asked for. Not from an
    // `@interfaceObject` though, its data has the interface's name in `__typename`.
    let asked_types = only_asked_types(graph, query_node)?;
    let narrowed = is_interface && !head_type.is_interface_object && !asked_types.is_empty();
    if narrowed {
        requirement.selection_set = SelectionSet {
            items: asked_types
                .into_iter()
                .map(|type_name| {
                    SelectionItem::InlineFragment(InlineFragmentSelection {
                        type_condition: type_name.to_string(),
                        selections: requirement.selection_set.clone(),
                        skip_if: None,
                        include_if: None,
                    })
                })
                .collect(),
        };
    }

    let tail_node_index = graph.get_edge_tail(&edge_index)?;
    let tail_node = graph.node(tail_node_index)?;
    let (output_type_name, subgraph_name) = match tail_node {
        Node::SubgraphType(t) => (&t.name, &t.subgraph),
        _ => return Err(FetchGraphError::ExpectedSubgraphType),
    };

    let fetch_step_index = ensure_fetch_step_for_subgraph(
        fetch_graph,
        parent_fetch_step_index,
        subgraph_name,
        input_type_name,
        output_type_name,
        response_path,
        Some(&requirement),
        conditions,
        created_from_requires,
        !is_interface && !head_type.is_interface_object,
    )?;
    fetch_graph
        .entity_sources
        .entry((fetch_step_index, tail_node_index))
        .or_insert(head_node_index);
    fetch_graph
        .entity_moves
        .entry((fetch_step_index, tail_node_index))
        .or_insert(edge_index);

    trace!(
        "adding input requirement '{}' to fetch step [{}]",
        requirement,
        fetch_step_index.index()
    );
    fetch_graph.add_input(
        fetch_step_index,
        input_type_name,
        &requirement.selection_set,
    )?;
    if is_interface && !narrowed {
        fetch_graph.touch(fetch_step_index);
    }
    let fetch_step = fetch_graph.get_step_data_mut(fetch_step_index)?;

    if is_interface && !narrowed {
        // We use `output_type_name` as there's no connection from `Interface` to `Object`,
        // it's always Object -> Interface.
        trace!(
            "adding input rewrite '... on {} {{ __typename }}' to '{}' of [{}]",
            output_type_name,
            output_type_name,
            fetch_step_index.index()
        );

        fetch_step.add_input_rewrite(FetchRewrite::ValueSetter(ValueSetter {
            path: vec![
                FetchNodePathSegment::typename_equals_from_type(output_type_name.to_string()),
                FetchNodePathSegment::Key("__typename".to_string()),
            ],
            set_value_to: output_type_name.to_string(),
        }));

        fetch_step
            .flags
            .insert(FetchStepFlags::USED_FOR_TYPE_CONDITION);
    }

    fetch_graph.add_output_typename(parent_fetch_step_index, response_path, fetch_path)?;

    // Make the fetch step a child of the parent fetch step
    trace!(
        "connecting fetch step to parent [{}] -> [{}]",
        parent_fetch_step_index.index(),
        fetch_step_index.index()
    );
    fetch_graph.connect(parent_fetch_step_index, fetch_step_index);

    process_requirements_for_fetch_steps(
        graph,
        fetch_graph,
        supergraph,
        override_context,
        query_node,
        parent_fetch_step_index,
        Some(fetch_step_index),
        conditions,
        response_path,
        fetch_path,
    )?;

    process_children_for_fetch_steps(
        graph,
        fetch_graph,
        supergraph,
        override_context,
        query_node,
        fetch_step_index,
        response_path,
        &MergePath::default(),
        requiring_fetch_step_index,
        conditions,
        created_from_requires,
    )
}

// TODO: simplfy args
#[allow(clippy::too_many_arguments)]
#[instrument(level = "trace",skip_all, fields(
  edge = graph.pretty_print_edge(edge_index, false),
  parent_fetch_step_index = parent_fetch_step_index.index(),
  requiring_fetch_step_index = requiring_fetch_step_index.map(|f| f.index()),
))]
fn process_interface_object_type_move_edge(
    graph: &Graph<'_>,
    fetch_graph: &mut FetchGraph<SingleTypeFetchStep>,
    supergraph: &SupergraphState,
    override_context: &PlannerOverrideContext,
    query_node: &QueryTreeNode,
    parent_fetch_step_index: NodeIndex,
    response_path: &MergePath,
    fetch_path: &MergePath,
    requiring_fetch_step_index: Option<NodeIndex>,
    edge_index: EdgeIndex,
    object_type_name: &str,
    conditions: Conditions<'_>,
    created_from_requires: bool,
) -> Result<Vec<NodeIndex>, FetchGraphError> {
    let edge = graph.edge(edge_index)?;
    let requirement = match edge {
        Edge::InterfaceObjectTypeMove(m) => TypeAwareSelection {
            selection_set: m.requirements.selection_set.clone(),
            type_name: m.requirements.type_name,
        },
        _ => {
            return Err(FetchGraphError::UnexpectedEdgeMove(
                "InterfaceObjectTypeMove".to_string(),
            ))
        }
    };
    let requirement = fetch_graph.read_at(response_path, &requirement);

    let head_node_index = graph.get_edge_head(&edge_index)?;
    let head_node = graph.node(head_node_index)?;
    let head_subgraph_name = match head_node {
        Node::SubgraphType(t) => &t.subgraph,
        _ => return Err(FetchGraphError::ExpectedSubgraphType),
    };

    let tail_node_index = graph.get_edge_tail(&edge_index)?;
    let tail_node = graph.node(tail_node_index)?;
    let (interface_type_name, subgraph_name) = match tail_node {
        Node::SubgraphType(t) => (&t.name, &t.subgraph),
        // todo: FetchGraphError::MissingSubgraphName(tail_node.clone())
        _ => return Err(FetchGraphError::ExpectedSubgraphType),
    };

    let step_for_children_index = ensure_fetch_step_for_subgraph(
        fetch_graph,
        parent_fetch_step_index,
        subgraph_name,
        object_type_name,
        interface_type_name,
        response_path,
        Some(&requirement),
        conditions,
        created_from_requires,
        false,
    )?;
    // it gets rewrites and keys, see `Replay`
    fetch_graph.touch(step_for_children_index);

    trace!(
        "adding input requirement '{}' to fetch step [{}]",
        requirement,
        step_for_children_index.index()
    );
    let requirement_node_index = query_node.requirements.first().unwrap().node_index;
    let key_to_reenter_subgraph = find_satisfiable_key(
        graph,
        fetch_graph,
        supergraph,
        override_context,
        requirement_node_index,
        requirement_node_index,
        false,
    )?
    .ok_or_else(no_satisfiable_key)?;
    let key_to_reenter_subgraph = &fetch_graph.read_at(response_path, key_to_reenter_subgraph);
    let step_for_children = fetch_graph.get_step_data_mut(step_for_children_index)?;
    step_for_children.input.add(&requirement.selection_set)?;
    trace!(
        "adding key '{}' to fetch step [{}]",
        key_to_reenter_subgraph,
        step_for_children_index.index()
    );
    step_for_children
        .input
        .add(&key_to_reenter_subgraph.selection_set)?;

    trace!(
        "adding input rewrite '... on {} {{ __typename }}' to '{}' of [{}]",
        interface_type_name,
        interface_type_name,
        step_for_children_index.index()
    );
    step_for_children.add_input_rewrite(FetchRewrite::ValueSetter(ValueSetter {
        path: vec![
            FetchNodePathSegment::typename_equals_from_type(interface_type_name.to_string()),
            FetchNodePathSegment::Key("__typename".to_string()),
        ],
        set_value_to: interface_type_name.to_string(),
    }));

    fetch_graph.add_output_typename(parent_fetch_step_index, response_path, fetch_path)?;

    // In all cases it's `__typename` that needs to be resolved by another subgraph.
    trace!("Creating a fetch step for requirement of @interfaceObject");
    let step_for_requirements_index = create_fetch_step_for_entity_call(
        fetch_graph,
        parent_fetch_step_index,
        head_subgraph_name,
        object_type_name,
        interface_type_name,
        response_path,
        conditions,
        false,
    );
    let step_for_requirements = fetch_graph.get_step_data_mut(step_for_requirements_index)?;
    trace!(
        "Adding {} to fetch([{}]).input",
        key_to_reenter_subgraph,
        step_for_requirements_index.index()
    );
    step_for_requirements
        .input
        .add(&key_to_reenter_subgraph.selection_set)?;

    //
    // Given `f0 { ... on User { f1 } }` where f1 is a field contributed by @interfaceObject,
    // the requirement to resolve `f1` with `User` type condition is a real value of `__typename` field.
    // - step_for_requirements    -> __typename
    // - step_for_children        -> f1
    // - parent                   -> f0
    //
    // parent -> step_for_requirements -> step_for_children
    //
    // f0 -> __typename -> f1
    //

    fetch_graph.connect(parent_fetch_step_index, step_for_requirements_index);

    trace!("Processing requirements");
    let leaf_fetch_step_indexes = process_query_node(
        graph,
        fetch_graph,
        supergraph,
        override_context,
        query_node.requirements.first().unwrap(),
        Some(step_for_requirements_index),
        response_path,
        &MergePath::default(),
        None,
        conditions,
        true,
    )?;

    if leaf_fetch_step_indexes.is_empty() {
        fetch_graph.connect(step_for_requirements_index, step_for_children_index);
    } else {
        for idx in leaf_fetch_step_indexes {
            fetch_graph.connect(idx, step_for_children_index);
        }
    }

    trace!("Processing children");
    process_children_for_fetch_steps(
        graph,
        fetch_graph,
        supergraph,
        override_context,
        query_node,
        step_for_children_index,
        response_path,
        &MergePath::default(),
        requiring_fetch_step_index,
        conditions,
        created_from_requires,
    )
}

// TODO: simplfy args
#[allow(clippy::too_many_arguments)]
#[instrument(level = "trace",skip_all, fields(
  subgraph = subgraph_name.0,
  type_name = type_name,
  parent_fetch_step_index = parent_fetch_step_index.index(),
))]
fn process_subgraph_entrypoint_edge(
    graph: &Graph<'_>,
    fetch_graph: &mut FetchGraph<SingleTypeFetchStep>,
    supergraph: &SupergraphState,
    override_context: &PlannerOverrideContext,
    query_node: &QueryTreeNode,
    parent_fetch_step_index: NodeIndex,
    subgraph_name: &SubgraphName,
    type_name: &str,
    operation_kind: OperationKind,
    conditions: Conditions<'_>,
    created_from_requires: bool,
) -> Result<Vec<NodeIndex>, FetchGraphError> {
    let fetch_step_index = create_fetch_step_for_root_move(
        fetch_graph,
        parent_fetch_step_index,
        subgraph_name,
        type_name,
        operation_kind,
        query_node.mutation_field_position,
        &MergePath::default(),
        conditions.without_current(),
    );

    process_children_for_fetch_steps(
        graph,
        fetch_graph,
        supergraph,
        override_context,
        query_node,
        fetch_step_index,
        &MergePath::default(),
        &MergePath::default(),
        None,
        conditions.without_current(),
        created_from_requires,
    )
}

#[allow(clippy::too_many_arguments)]
#[instrument(level = "trace",skip_all, fields(
  parent_fetch_step_index = parent_fetch_step_index.index(),
))]
fn process_subgraph_reentry(
    graph: &Graph<'_>,
    fetch_graph: &mut FetchGraph<SingleTypeFetchStep>,
    supergraph: &SupergraphState,
    edge_index: EdgeIndex,
    override_context: &PlannerOverrideContext,
    query_node: &QueryTreeNode,
    parent_fetch_step_index: NodeIndex,
    created_from_requires: bool,
    reentry_move: &ReentryMove,
    response_path: &MergePath,
    fetch_path: &MergePath,
    requiring_fetch_step_index: Option<NodeIndex>,
    conditions: Conditions<'_>,
) -> Result<Vec<NodeIndex>, FetchGraphError> {
    let condition = conditions.current;
    let node_index = graph.get_edge_tail(&edge_index).unwrap();
    let node = graph.node(node_index).unwrap();

    let (type_name, subgraph_name) = match node {
        Node::SubgraphType(subgraph_type) => (&subgraph_type.name, &subgraph_type.subgraph),
        _ => return Err(FetchGraphError::ExpectedSubgraphType),
    };

    // The re-entry field is part of the parent's response shape, but its children resolve in a
    // separate (re-entered) fetch. Record the field in the parent output with a `__typename`
    // so the parent operation stays valid.
    //
    let field_condition = fetch_graph.condition_to_apply(
        parent_fetch_step_index,
        requiring_fetch_step_index,
        fetch_path,
        condition,
    );
    let field_alias = fetch_graph.alias_at(
        response_path,
        query_node,
        &reentry_move.name,
        created_from_requires,
    );
    fetch_graph.add_output(
        parent_fetch_step_index,
        response_path,
        fetch_path,
        SelectionSet {
            items: vec![SelectionItem::Field(FieldSelection {
                name: reentry_move.name.to_string(),
                alias: field_alias.clone(),
                selections: SelectionSet {
                    items: vec![SelectionItem::Field(FieldSelection::new_typename())],
                },
                arguments: query_node.selection_arguments().cloned(),
                skip_if: field_condition.as_ref().and_then(Condition::to_skip_if),
                include_if: field_condition.as_ref().and_then(Condition::to_include_if),
                omit_from_response: false,
            })],
        },
    )?;

    let segment_field = Rc::new(FieldPathSegment::new(
        reentry_move.name.to_string(),
        field_alias,
    ));
    let child_response_path = path_into_field(
        response_path,
        Segment::Field(segment_field, 0, None),
        reentry_move.list_depth,
    );

    let fetch_step_index = create_fetch_step_for_root_move(
        fetch_graph,
        parent_fetch_step_index,
        subgraph_name,
        type_name,
        OperationKind::Query,
        query_node.mutation_field_position,
        &child_response_path,
        conditions,
    );

    process_children_for_fetch_steps(
        graph,
        fetch_graph,
        supergraph,
        override_context,
        query_node,
        fetch_step_index,
        &child_response_path,
        &MergePath::default(),
        requiring_fetch_step_index,
        conditions.without_current(),
        created_from_requires,
    )
}

// TODO: simplfy args
#[allow(clippy::too_many_arguments)]
#[instrument(level = "trace",skip_all, fields(
  parent_fetch_step_index = parent_fetch_step_index.index(),
  requiring_fetch_step_index = requiring_fetch_step_index.map(|f| f.index()),
  type_name = target_type_name,
  response_path = response_path.to_string(),
  fetch_path = fetch_path.to_string(),
))]
fn process_selfie_edge(
    graph: &Graph<'_>,
    fetch_graph: &mut FetchGraph<SingleTypeFetchStep>,
    supergraph: &SupergraphState,
    override_context: &PlannerOverrideContext,
    query_node: &QueryTreeNode,
    parent_fetch_step_index: NodeIndex,
    requiring_fetch_step_index: Option<NodeIndex>,
    response_path: &MergePath,
    fetch_path: &MergePath,
    target_type_name: &String,
    conditions: Conditions<'_>,
    created_from_requires: bool,
) -> Result<Vec<NodeIndex>, FetchGraphError> {
    let condition = conditions.current;
    let condition_on_step = condition.is_some_and(|c| {
        fetch_graph.is_condition_on_step(parent_fetch_step_index, requiring_fetch_step_index, c)
    });

    trace!(
        "starting an inline fragment for type '{}' to fetch step [{}]",
        parent_fetch_step_index.index(),
        target_type_name,
    );
    fetch_graph.add_output(
        parent_fetch_step_index,
        response_path,
        fetch_path,
        SelectionSet {
            items: vec![SelectionItem::InlineFragment(InlineFragmentSelection {
                type_condition: target_type_name.clone(),
                selections: SelectionSet::default(),
                skip_if: condition.and_then(|c| {
                    if condition_on_step {
                        None
                    } else {
                        c.to_skip_if()
                    }
                }),
                include_if: condition.and_then(|c| {
                    if condition_on_step {
                        None
                    } else {
                        c.to_include_if()
                    }
                }),
            })],
        },
    )?;

    let segment_condition = if condition_on_step {
        None
    } else {
        condition.cloned()
    };
    let child_response_path = response_path.push(Segment::TypeCondition(
        Rc::new(BTreeSet::from([target_type_name.clone()])),
        segment_condition.clone(),
    ));
    let child_fetch_path = fetch_path.push(Segment::TypeCondition(
        Rc::new(BTreeSet::from([target_type_name.clone()])),
        segment_condition,
    ));

    process_children_for_fetch_steps(
        graph,
        fetch_graph,
        supergraph,
        override_context,
        query_node,
        parent_fetch_step_index,
        &child_response_path,
        &child_fetch_path,
        requiring_fetch_step_index,
        conditions,
        created_from_requires,
    )
}

// TODO: simplfy args
#[allow(clippy::too_many_arguments)]
#[instrument(level = "trace",skip_all, fields(
  parent_fetch_step_index = parent_fetch_step_index.index(),
  requiring_fetch_step_index = requiring_fetch_step_index.map(|f| f.index()),
  type_name = target_type_name,
  response_path = response_path.to_string(),
  fetch_path = fetch_path.to_string()
))]
fn process_abstract_edge(
    graph: &Graph<'_>,
    fetch_graph: &mut FetchGraph<SingleTypeFetchStep>,
    supergraph: &SupergraphState,
    override_context: &PlannerOverrideContext,
    query_node: &QueryTreeNode,
    parent_fetch_step_index: NodeIndex,
    requiring_fetch_step_index: Option<NodeIndex>,
    response_path: &MergePath,
    fetch_path: &MergePath,
    target_type_name: &String,
    conditions: Conditions<'_>,
    created_from_requires: bool,
) -> Result<Vec<NodeIndex>, FetchGraphError> {
    let condition = conditions.current;
    // Everything under the fragment has to stay behind its condition. The condition goes down to
    // the children and each one applies it, but a child with a condition of its own carries only
    // that one, like `name` in `... on Dog @include(if: $x) { name @skip(if: $y) }` would lose
    // `$x`. So when a child has another condition, the fragment keeps its own, like a field does.
    // Only the children need checking. Deeper down, whatever they sit under already applies it:
    // a field carries it, an entity call is skipped with it, and a fragment checks again here.
    let child_has_other_condition = condition.is_some_and(|condition| {
        query_node
            .children
            .iter()
            .any(|child| child.condition.as_ref().is_some_and(|c| c != condition))
    });
    let segment_condition = fetch_graph.condition_to_apply(
        parent_fetch_step_index,
        requiring_fetch_step_index,
        fetch_path,
        condition.filter(|_| child_has_other_condition),
    );

    trace!(
        "adding output field '__typename' and starting an inline fragment for type '{}' to fetch step [{}]",
        parent_fetch_step_index.index(),
        target_type_name,
    );

    fetch_graph.add_output(
        parent_fetch_step_index,
        response_path,
        fetch_path,
        SelectionSet {
            items: vec![
                SelectionItem::Field(FieldSelection::new_typename()),
                SelectionItem::InlineFragment(InlineFragmentSelection {
                    type_condition: target_type_name.clone(),
                    selections: SelectionSet::default(),
                    skip_if: segment_condition.as_ref().and_then(Condition::to_skip_if),
                    include_if: segment_condition
                        .as_ref()
                        .and_then(Condition::to_include_if),
                }),
            ],
        },
    )?;

    let child_response_path = response_path.push(Segment::TypeCondition(
        Rc::new(BTreeSet::from([target_type_name.clone()])),
        segment_condition.clone(),
    ));
    let child_fetch_path = fetch_path.push(Segment::TypeCondition(
        Rc::new(BTreeSet::from([target_type_name.clone()])),
        segment_condition,
    ));

    process_children_for_fetch_steps(
        graph,
        fetch_graph,
        supergraph,
        override_context,
        query_node,
        parent_fetch_step_index,
        &child_response_path,
        &child_fetch_path,
        requiring_fetch_step_index,
        conditions,
        created_from_requires,
    )
}

// TODO: simplfy args
#[allow(clippy::too_many_arguments)]
#[instrument(level = "trace",skip_all, fields(
  parent_fetch_step_index = parent_fetch_step_index.index(),
  requiring_fetch_step_index = requiring_fetch_step_index.map(|f| f.index()),
  type_name = field_move.type_name,
  field = field_move.name,
  alias = query_node.selection_attributes.as_ref().and_then(|v| v.alias.as_ref()),
  arguments = query_node.selection_attributes.as_ref().and_then(|v| v.arguments.as_ref()).map(|v| format!("{}", v)),
  leaf = field_move.is_leaf,
  list_depth = field_move.list_depth,
  response_path = response_path.to_string(),
  fetch_path = fetch_path.to_string(),
))]
fn process_plain_field_edge(
    graph: &Graph<'_>,
    fetch_graph: &mut FetchGraph<SingleTypeFetchStep>,
    supergraph: &SupergraphState,
    override_context: &PlannerOverrideContext,
    query_node: &QueryTreeNode,
    parent_fetch_step_index: NodeIndex,
    requiring_fetch_step_index: Option<NodeIndex>,
    response_path: &MergePath,
    fetch_path: &MergePath,
    field_move: &FieldMove,
    conditions: Conditions<'_>,
    created_from_requires: bool,
) -> Result<Vec<NodeIndex>, FetchGraphError> {
    let condition = conditions.current;
    if let Some(requiring_fetch_step_index) = requiring_fetch_step_index {
        trace!(
            "connecting parent fetch step [{}] to requiring fetch step [{}]",
            parent_fetch_step_index.index(),
            requiring_fetch_step_index.index()
        );
        fetch_graph.connect(parent_fetch_step_index, requiring_fetch_step_index);
    }

    let segment_condition = fetch_graph.condition_to_apply(
        parent_fetch_step_index,
        requiring_fetch_step_index,
        fetch_path,
        condition,
    );

    let field_alias = fetch_graph.alias_at(
        response_path,
        query_node,
        field_move.name,
        created_from_requires,
    );
    trace!(
        "adding output field '{}' to fetch step [{}]",
        field_move.name,
        parent_fetch_step_index.index()
    );

    fetch_graph.add_output(
        parent_fetch_step_index,
        response_path,
        fetch_path,
        SelectionSet {
            items: vec![SelectionItem::Field(FieldSelection {
                name: field_move.name.to_string(),
                alias: field_alias.clone(),
                selections: SelectionSet::default(),
                arguments: query_node.selection_arguments().cloned(),
                skip_if: segment_condition.as_ref().and_then(Condition::to_skip_if),
                include_if: segment_condition
                    .as_ref()
                    .and_then(Condition::to_include_if),
                omit_from_response: false,
            })],
        },
    )?;

    let segment_args_hash = query_node
        .selection_arguments()
        .map(|a| a.hash_u64())
        .unwrap_or(0);
    let segment_field = Rc::new(FieldPathSegment::new(
        field_move.name.to_string(),
        field_alias,
    ));
    let child_response_path = path_into_field(
        response_path,
        Segment::Field(
            segment_field.clone(),
            segment_args_hash,
            segment_condition.clone(),
        ),
        field_move.list_depth,
    );
    let child_fetch_path = path_into_field(
        fetch_path,
        Segment::Field(segment_field, segment_args_hash, segment_condition),
        field_move.list_depth,
    );

    process_children_for_fetch_steps(
        graph,
        fetch_graph,
        supergraph,
        override_context,
        query_node,
        parent_fetch_step_index,
        &child_response_path,
        &child_fetch_path,
        requiring_fetch_step_index,
        conditions,
        created_from_requires,
    )
}

fn context_selection_rewrites(
    items: &[SelectionItem],
    path: &[FetchNodePathSegment],
    variable: &str,
    rewrites: &mut Vec<KeyRenamer>,
) {
    for item in items {
        let mut path = path.to_vec();
        match item {
            SelectionItem::Field(field) => {
                path.push(FetchNodePathSegment::Key(
                    field.selection_identifier().to_string(),
                ));
                if field.selections.items.is_empty() {
                    rewrites.push(KeyRenamer {
                        path,
                        rename_key_to: variable.to_string(),
                    });
                } else {
                    context_selection_rewrites(&field.selections.items, &path, variable, rewrites);
                }
            }
            SelectionItem::InlineFragment(fragment) => {
                path.push(FetchNodePathSegment::typename_equals_from_type(
                    fragment.type_condition.clone(),
                ));
                context_selection_rewrites(&fragment.selections.items, &path, variable, rewrites);
            }
            SelectionItem::FragmentSpread(_) => {}
        }
    }
}

/// Where the objects go back into the subgraph of a `@requires` field, and with which key.
struct Reentry<'a> {
    /// The step the key is read from, and that the field's fetches wait for.
    from: NodeIndex,
    /// Which subgraph (and type) the objects were loaded from. `None` if we don't know.
    holder: Option<NodeIndex>,
    key: &'a TypeAwareSelection<'a>,
    /// Set when the key is in the response already: the fetches that put it there, to wait for
    /// instead of asking for the key.
    key_fetched_by: Vec<NodeIndex>,
}

/// The objects go back into the field's subgraph with one of its keys. When the subgraph they
/// came from resolves one, it fetches it too. When it doesn't, like with `@key(fields: "id sku")`
/// and `sku` coming from another subgraph, there's the entity call we're in: it was made with a
/// key of this subgraph, so that key is in the response before it runs, whichever fetches put it
/// there. We go in with the same key and wait for the same fetches. The call itself is the last
/// resort: it's in this subgraph, so it resolves its own keys and returns one.
fn decide_reentry<'a>(
    graph: &'a Graph<'a>,
    fetch_graph: &mut FetchGraph<SingleTypeFetchStep>,
    supergraph: &'a SupergraphState,
    override_context: &'a PlannerOverrideContext,
    head_node_index: NodeIndex,
    response_path: &MergePath,
    parent_fetch_step_index: NodeIndex,
) -> Result<Reentry<'a>, FetchGraphError> {
    let head_type_name = match graph.node(head_node_index)? {
        Node::SubgraphType(t) => t.name,
        _ => return Err(FetchGraphError::ExpectedSubgraphType),
    };
    let parent_fetch_step = fetch_graph.get_step_data(parent_fetch_step_index)?;
    // In case of a field with `@requires`, the parent will be the current subgraph we're in.
    let real_parent_fetch_step_index = match !parent_fetch_step
        .output
        .is_selecting_definition(head_type_name)
        // The parent is an entity call of the same type, but the field sits deeper in its
        // output, like `User.other: User`. The keys can go right there.
        || parent_fetch_step.response_path.len() < response_path.len()
    {
        // If the parent's output resolves a different type, then it's a root type.
        // We can use that as a parent.
        true => parent_fetch_step_index,
        // If the parent's output resolves the same type, it means we're in an entity call.
        // We need to move up, as the entity call was created to fetch regular fields of the type
        // (those without @requires).
        //
        // Example: Fetch Step was created to get `baz`
        // {
        //   foo
        //   bar @requires(fields: "foo")
        //   baz
        // }
        //
        // We need to go to the step that holds the object that entity call extends. Not to
        // "the parent of the parent": a step with `@requires` waits for every step that fetches
        // one of the required fields, so it can have many parents.
        false => *fetch_graph
            .entity_owners
            .get(&parent_fetch_step_index)
            .ok_or_else(|| FetchGraphError::MissingEntityOwner(parent_fetch_step_index.index()))?,
    };
    let holder = if real_parent_fetch_step_index == parent_fetch_step_index {
        Some(head_node_index)
    } else {
        fetch_graph
            .entity_sources
            .get(&(parent_fetch_step_index, head_node_index))
            .copied()
    };
    let own_key = |fetch_graph: &mut FetchGraph<SingleTypeFetchStep>| {
        find_satisfiable_key(
            graph,
            fetch_graph,
            supergraph,
            override_context,
            head_node_index,
            head_node_index,
            true,
        )?
        .ok_or_else(no_satisfiable_key)
    };
    let reentry = |from, holder, key| Reentry {
        from,
        holder,
        key,
        key_fetched_by: Vec::new(),
    };
    match holder {
        Some(holder_node) if real_parent_fetch_step_index != parent_fetch_step_index => {
            if let Some(key) = find_satisfiable_key(
                graph,
                fetch_graph,
                supergraph,
                override_context,
                head_node_index,
                holder_node,
                true,
            )? {
                Ok(reentry(real_parent_fetch_step_index, holder, key))
            } else if let Some(key) =
                key_of_entity_call(graph, fetch_graph, parent_fetch_step_index, head_node_index)?
            {
                Ok(Reentry {
                    from: real_parent_fetch_step_index,
                    holder: None,
                    key,
                    key_fetched_by: fetch_graph
                        .parents_of(parent_fetch_step_index)
                        .map(|edge| edge.source())
                        .collect(),
                })
            } else {
                Ok(reentry(
                    parent_fetch_step_index,
                    Some(head_node_index),
                    own_key(fetch_graph)?,
                ))
            }
        }
        // We don't know the subgraph the objects came from, or we're in it already.
        _ => Ok(reentry(
            real_parent_fetch_step_index,
            holder,
            own_key(fetch_graph)?,
        )),
    }
}

#[allow(clippy::too_many_arguments)]
#[instrument(level = "trace",skip_all, fields(
  parent_fetch_step_index = parent_fetch_step_index.index(),
  requiring_fetch_step_index = requiring_fetch_step_index.map(|s| s.index()),
))]
fn process_requires_field_edge(
    graph: &Graph<'_>,
    fetch_graph: &mut FetchGraph<SingleTypeFetchStep>,
    supergraph: &SupergraphState,
    override_context: &PlannerOverrideContext,
    query_node: &QueryTreeNode,
    parent_fetch_step_index: NodeIndex,
    response_path: &MergePath,
    requiring_fetch_step_index: Option<NodeIndex>,
    field_move: &FieldMove,
    edge_index: EdgeIndex,
    conditions: Conditions<'_>,
    created_from_requires: bool,
) -> Result<Vec<NodeIndex>, FetchGraphError> {
    let requires = field_move.requirements.as_ref();

    let head_node_index = graph.get_edge_head(&edge_index)?;
    let head_node = graph.node(head_node_index)?;
    let (head_type_name, head_subgraph_name, is_interface_object) = match head_node {
        Node::SubgraphType(t) => (t.name, &t.subgraph, &t.is_interface_object),
        _ => return Err(FetchGraphError::ExpectedSubgraphType),
    };

    let reentry = decide_reentry(
        graph,
        fetch_graph,
        supergraph,
        override_context,
        head_node_index,
        response_path,
        parent_fetch_step_index,
    )?;
    let requires_input = requires.map(|requires| fetch_graph.read_at(response_path, requires));
    let key_input = fetch_graph.read_at(response_path, reentry.key);
    let key_output = fetch_graph
        .response_keys
        .output(response_path, &reentry.key.selection_set);
    let field_alias = fetch_graph.alias_at(
        response_path,
        query_node,
        field_move.name,
        created_from_requires,
    );

    // A scalar with `@requires` that's been planned at this spot already, under the same
    // conditions, has its value coming: whatever needs it now waits for that step.
    let planned =
        (query_node.children.is_empty() && field_move.context_arguments.is_empty()).then(|| {
            PlannedRequires {
                edge: edge_index,
                response_path: response_path.clone(),
                response_key: field_alias.clone(),
                arguments: query_node.selection_arguments().cloned(),
                conditions: conditions.from_root.to_vec(),
                current_condition: conditions.current.cloned(),
            }
        });
    if let Some(&step) = planned
        .as_ref()
        .and_then(|planned| fetch_graph.planned_requires.get(planned))
    {
        return Ok(vec![step]);
    }

    // Given `f0 { f1 @requires(fields: f2) }`, the field fetch gets `f1` with `f2` as input, and
    // the requirements fetch (or the ones it leads to) gets `f2`, from where `f0` is:
    // f0 -> f2 -> f1
    // and with `f2 @requires(fields: f3)`: f0 -> f3 -> f2 -> f1, and so on.
    trace!("Creating a fetch step for the field with @requires");
    let field_fetch = if field_move.context_arguments.is_empty() {
        ensure_fetch_step_for_requirement(
            fetch_graph,
            reentry.from,
            head_subgraph_name,
            head_type_name,
            response_path,
            conditions,
            requires_input
                .as_ref()
                .ok_or(FetchGraphError::MissingRequires)?,
        )?
    } else {
        create_fetch_step_for_entity_call(
            fetch_graph,
            reentry.from,
            head_subgraph_name,
            head_type_name,
            head_type_name,
            response_path,
            conditions,
            true,
        )
    };
    // a step that was there gets the field and its input, see `Replay`
    fetch_graph.touch(field_fetch);
    if let Some(holder_node) = reentry.holder {
        fetch_graph
            .entity_sources
            .entry((field_fetch, head_node_index))
            .or_insert(holder_node);
    }

    let arguments = add_context_arguments(
        fetch_graph,
        query_node,
        field_move,
        edge_index,
        response_path,
        field_fetch,
    )?;
    let fetch_arguments_hash = arguments
        .as_ref()
        .map(|arguments| arguments.hash_u64())
        .unwrap_or(0);
    let field_fetch_step = fetch_graph.get_step_data_mut(field_fetch)?;

    field_fetch_step.add_output_at(
        response_path,
        &MergePath::default(),
        SelectionSet {
            items: vec![SelectionItem::Field(FieldSelection {
                name: field_move.name.to_string(),
                alias: field_alias.clone(),
                selections: SelectionSet::default(),
                arguments,
                skip_if: None,
                include_if: None,
                omit_from_response: false,
            })],
        },
    )?;

    if *is_interface_object {
        trace!(
            "adding input rewrite '... on {} {{ __typename }}' to '{}' of [{}]",
            head_type_name,
            head_type_name,
            field_fetch.index(),
        );
        field_fetch_step.add_input_rewrite(FetchRewrite::ValueSetter(ValueSetter {
            path: vec![
                FetchNodePathSegment::typename_equals_from_type(head_type_name.to_string()),
                FetchNodePathSegment::Key("__typename".to_string()),
            ],
            set_value_to: head_type_name.to_string(),
        }));
    }

    if let Some(requires_input) = &requires_input {
        trace!(
            "Adding {} to fetch([{}]).input (requires)",
            requires_input,
            field_fetch.index()
        );
        field_fetch_step.input.add(&requires_input.selection_set)?;
    }
    trace!(
        "Adding {} to fetch([{}]).input (key re-enter)",
        key_input,
        field_fetch.index()
    );
    field_fetch_step.input.add(&key_input.selection_set)?;

    let reentry_step = fetch_graph.get_step_data_mut(reentry.from)?;

    let key_to_reenter_at = if reentry_step.response_path.len() > response_path.len() {
        return Err(FetchGraphError::Internal(
            "Response path is longer than expected".to_string(),
        ));
    } else {
        response_path.slice_from(reentry_step.response_path.len())
    };

    trace!(
        "Adding {} to fetch([{}]).output at path {}",
        reentry.key,
        reentry.from.index(),
        key_to_reenter_at
    );

    // A key that's already in the response only needs us to wait for the fetches that put it there.
    if reentry.key_fetched_by.is_empty() {
        fetch_graph.add_output(reentry.from, response_path, &key_to_reenter_at, key_output)?;
        fetch_graph.add_output_typename(reentry.from, response_path, &key_to_reenter_at)?;
    }
    for &step in &reentry.key_fetched_by {
        fetch_graph.connect(step, field_fetch);
    }

    // On a `@provides` path, some required fields only exist on that path. The subgraph can't
    // return them from `_entities`, so the fetch that went through the path gets them, and
    // whatever needs another fetch starts from there too.
    let requirements_start_at_reentry = requires.is_none()
        || head_node.is_using_provides()
        || can_leave_from_holder(
            graph,
            fetch_graph,
            supergraph,
            override_context,
            query_node,
            field_move,
            &reentry,
            *is_interface_object,
        );
    let (requirements_fetch, requirements_fetch_path) = if requirements_start_at_reentry {
        (reentry.from, key_to_reenter_at)
    } else {
        let requirements_fetch = add_requirements_fetch(
            graph,
            fetch_graph,
            supergraph,
            override_context,
            query_node,
            &reentry,
            &key_input,
            head_node_index,
            head_subgraph_name,
            head_type_name,
            response_path,
            &key_to_reenter_at,
            conditions,
        )?;
        (requirements_fetch, MergePath::default())
    };

    let segment_field = Rc::new(FieldPathSegment::new(
        field_move.name.to_string(),
        field_alias,
    ));
    let child_response_path = path_into_field(
        response_path,
        Segment::Field(segment_field.clone(), fetch_arguments_hash, None),
        field_move.list_depth,
    );
    let child_fetch_path = path_into_field(
        &MergePath::default(),
        Segment::Field(segment_field, fetch_arguments_hash, None),
        field_move.list_depth,
    );

    trace!("Processing requirements");
    let leaf_fetch_step_indexes = if requires.is_some() {
        process_query_node(
            graph,
            fetch_graph,
            supergraph,
            override_context,
            query_node
                .requirements
                .first()
                .ok_or(FetchGraphError::MissingRequires)?,
            Some(requirements_fetch),
            response_path,
            &requirements_fetch_path,
            None,
            conditions,
            true,
        )?
    } else {
        Vec::new()
    };

    // The field fetch waits for the whole chain of fetches the requirements make, or for the
    // requirements fetch when that's all of it.
    if leaf_fetch_step_indexes.is_empty() {
        trace!("Connecting fetch that pulls requirements with fetch that resolves the field with @requires");
        fetch_graph.connect(requirements_fetch, field_fetch);
    } else {
        trace!("Connecting leaf fetches of requirements to fetch with @requires");
        for idx in leaf_fetch_step_indexes {
            fetch_graph.connect(idx, field_fetch);
        }
    }

    if let Some(planned) = planned {
        fetch_graph.planned_requires.insert(planned, field_fetch);
    }

    trace!("Processing children");
    process_children_for_fetch_steps(
        graph,
        fetch_graph,
        supergraph,
        override_context,
        query_node,
        field_fetch,
        &child_response_path,
        &child_fetch_path,
        requiring_fetch_step_index,
        conditions,
        created_from_requires,
    )
}

/// Turns the field's `@fromContext` arguments into variables of `field_fetch`, waiting for the
/// steps that produce them. Returns the arguments the field is asked with.
fn add_context_arguments(
    fetch_graph: &mut FetchGraph<SingleTypeFetchStep>,
    query_node: &QueryTreeNode,
    field_move: &FieldMove,
    edge_index: EdgeIndex,
    response_path: &MergePath,
    field_fetch: NodeIndex,
) -> Result<Option<ArgumentsMap>, FetchGraphError> {
    let mut arguments = query_node
        .selection_arguments()
        .cloned()
        .unwrap_or_default();
    for requirement in query_node.resolved_context_requirements() {
        let argument = &field_move.context_arguments[requirement.argument];
        // a contextual field inside a @requires or context selection only sees ancestors from
        // the root of that selection down, that root is where the selection gets planned and
        // where its context producers get registered. a context set above the root has no
        // ancestor path or producer here, so we error out. this is on purpose and matches
        // the federation "spec", which resolves those selections as fresh paths from the requiring
        // entity and treats a context above it as not set. composition reports such a query as
        // unsatisfiable, so a valid supergraph never gets here
        let ancestor_index = response_path
            .inner
            .iter()
            .enumerate()
            .rev()
            .filter(|(_, segment)| matches!(segment, Segment::Field(..)))
            .nth(requirement.levels - 1)
            .map(|(index, _)| index)
            .ok_or(FetchGraphError::UnsupportedContextInRequirement)?;
        let ancestor_path = MergePath::new(response_path.inner[..ancestor_index].to_vec());
        let producers = fetch_graph
            .context_producers
            .get(&(ancestor_path.clone(), (edge_index, requirement.argument)))
            .cloned()
            .ok_or(FetchGraphError::UnsupportedContextInRequirement)?;
        for producer in producers {
            let id = fetch_graph.get_step_data(producer)?.id;
            fetch_graph.connect(producer, field_fetch);
            fetch_graph
                .get_step_data_mut(field_fetch)?
                .context_producers
                .insert(id);
        }
        // same field, same ancestor, same value, so same variable. two copies of the field, like
        // one under `@include` and one under `@skip`, then have the same arguments and merge
        let key = ContextVariableKey {
            ancestor_path: ancestor_path.clone(),
            edge_index,
            argument: requirement.argument,
            selection: requirement.selection,
            levels: requirement.levels,
        };
        let variable = match fetch_graph.context_variables.get(&key) {
            Some(name) => name.clone(),
            None => {
                let name = loop {
                    let name = format!("__context_{}", fetch_graph.context_variable_counter);
                    fetch_graph.context_variable_counter += 1;
                    if !fetch_graph.operation_variables.contains(&name) {
                        break name;
                    }
                };
                fetch_graph.context_variables.insert(key, name.clone());
                name
            }
        };
        arguments.add_argument(argument.name.clone(), Value::Variable(variable.clone()));
        let selection = &argument.selections[requirement.selection];
        let output = fetch_graph
            .response_keys
            .output(&ancestor_path, &selection.selection_set);
        let prefix = vec![FetchNodePathSegment::Parent; requirement.levels];
        let mut field_prefix = prefix.clone();
        field_prefix.push(FetchNodePathSegment::typename_equals_from_type(
            selection.type_name.to_string(),
        ));
        let mut rewrites = Vec::new();
        for item in &output.items {
            // root fields read from the matched ancestor type, root fragments carry their own type
            let path = match item {
                SelectionItem::Field(_) => &field_prefix,
                _ => &prefix,
            };
            context_selection_rewrites(std::slice::from_ref(item), path, &variable, &mut rewrites);
        }
        let step = fetch_graph.get_step_data_mut(field_fetch)?;
        if step
            .context_variable_definitions
            .iter()
            .any(|definition| definition.name == variable)
        {
            continue;
        }
        step.context_rewrites
            .get_or_insert_default()
            .extend(rewrites);
        step.context_variable_definitions.push(VariableDefinition {
            name: variable,
            variable_type: argument.type_in_graph.clone(),
            default_value: None,
        });
    }
    Ok((!arguments.is_empty()).then_some(arguments))
}

/// The step that holds the objects goes back into their subgraph with the key, and the
/// requirements leave from there. When all of them leave right away, to subgraphs whose keys the
/// subgraph the objects came from resolves, that step would fetch nothing of its own: they can
/// leave from the step that holds the objects instead, like other entity calls do, and share
/// fetches with them.
#[allow(clippy::too_many_arguments)]
fn can_leave_from_holder(
    graph: &Graph<'_>,
    fetch_graph: &mut FetchGraph<SingleTypeFetchStep>,
    supergraph: &SupergraphState,
    override_context: &PlannerOverrideContext,
    query_node: &QueryTreeNode,
    field_move: &FieldMove,
    reentry: &Reentry<'_>,
    is_interface_object: bool,
) -> bool {
    let (Some(holder), Some(requirements)) = (reentry.holder, query_node.requirements.first())
    else {
        return false;
    };
    if field_move.requirements.is_none()
        || !reentry.key_fetched_by.is_empty()
        || !field_move.context_arguments.is_empty()
        || is_interface_object
        || requirements.children.is_empty()
    {
        return false;
    }
    requirements.children.iter().all(|child| {
        let Some(edge_index) = child.edge_from_parent else {
            return false;
        };
        let edge_ref = graph.edge_reference(edge_index);
        matches!(edge_ref.weight(), Edge::EntityMove(_))
            && satisfiable_from(
                graph,
                fetch_graph,
                supergraph,
                override_context,
                &edge_ref,
                holder,
            )
            // on purpose: this only checks whether a shortcut works, and if the check fails the
            // requirements just take the usual way, which reports real errors
            .unwrap_or(false)
    })
}

/// A new entity call back into the field's subgraph that the requirements leave from. Passes
/// along the keys they need to reach their subgraphs when the response has them already, or the
/// subgraph the objects came from can fetch them too.
#[allow(clippy::too_many_arguments)]
fn add_requirements_fetch(
    graph: &Graph<'_>,
    fetch_graph: &mut FetchGraph<SingleTypeFetchStep>,
    supergraph: &SupergraphState,
    override_context: &PlannerOverrideContext,
    query_node: &QueryTreeNode,
    reentry: &Reentry<'_>,
    key_input: &TypeAwareSelection<'_>,
    head_node_index: NodeIndex,
    subgraph_name: &SubgraphName,
    type_name: &str,
    response_path: &MergePath,
    key_to_reenter_at: &MergePath,
    conditions: Conditions<'_>,
) -> Result<NodeIndex, FetchGraphError> {
    trace!("Creating a fetch step for requirement of @requires");
    let requirements_fetch = create_fetch_step_for_entity_call(
        fetch_graph,
        reentry.from,
        subgraph_name,
        type_name,
        type_name,
        response_path,
        conditions,
        true,
    );
    trace!(
        "Adding {} to fetch([{}]).input",
        key_input,
        requirements_fetch.index()
    );
    fetch_graph
        .get_step_data_mut(requirements_fetch)?
        .input
        .add(&key_input.selection_set)?;
    fetch_graph.connect(reentry.from, requirements_fetch);
    for &step in &reentry.key_fetched_by {
        fetch_graph.connect(step, requirements_fetch);
    }

    if let Some(holder_node) = reentry.holder {
        fetch_graph
            .entity_sources
            .entry((requirements_fetch, head_node_index))
            .or_insert(holder_node);
    }

    // The requirements need another key to reach their subgraph. When it's already in the
    // response, a part of the key we go back in with, or the subgraph the objects came from can
    // fetch it too, we pass it along instead of asking this subgraph for it.
    let mut jump_keys = Vec::new();
    if reentry.holder.is_some() || !reentry.key_fetched_by.is_empty() {
        let requirements = query_node
            .requirements
            .first()
            .ok_or(FetchGraphError::MissingRequires)?;
        let jumps: Vec<EdgeIndex> = requirements
            .children
            .iter()
            .filter_map(|child| child.edge_from_parent)
            .collect();

        for edge_ref in graph
            .edges_from(requirements.node_index)
            .filter(|edge_ref| jumps.contains(&edge_ref.id()))
        {
            let Edge::EntityMove(entity_move) = edge_ref.weight() else {
                continue;
            };
            let key = &entity_move.requirements;
            if !reentry.key_fetched_by.is_empty() {
                if key.type_name == reentry.key.type_name
                    && reentry.key.selection_set.contains(&key.selection_set)
                {
                    jump_keys.push((key, false));
                }
                continue;
            }
            let Some(holder_node) = reentry.holder else {
                continue;
            };
            // Only what the holder's subgraph resolves on that spot, no jumps.
            if satisfiable_from(
                graph,
                fetch_graph,
                supergraph,
                override_context,
                &edge_ref,
                holder_node,
            )? {
                jump_keys.push((key, true));
            }
        }
    }
    for (key, fetch_it) in jump_keys {
        let input = fetch_graph.read_at(response_path, key);
        fetch_graph
            .get_step_data_mut(requirements_fetch)?
            .input
            .add(&input.selection_set)?;
        if !fetch_it {
            continue;
        }
        let output = fetch_graph
            .response_keys
            .output(response_path, &key.selection_set);
        fetch_graph.add_output(reentry.from, response_path, key_to_reenter_at, output)?;
    }

    Ok(requirements_fetch)
}

/// A field marked with `@requires` tells us that in order to resolve it,
/// the subgraph needs data from another subgraph.
///
/// This interaction ALWAYS involves an entity call,
/// but the trigger for resolving the field differs:
///
/// 1.  Cross-Subgraph: If the field annotated with `@requires` is being
///     fetched after an `EntityMove` brought us to the current subgraph from another,
///     the `EntityMove` itself already handled fetching the necessary `@key` fields.
///     We don't need to think about it, we just need to add the field set of `@requires(fields:)`
///     to the `FetchStep` input.
///
/// 2.  Root: If the field annotated with `@requires` is being fetched locally,
///     because the query started from the root type,
///     the `EntityMove` is needed, but the data to satisfy the key fields is all local.
///     The subgraph effectively makes an "internal" entity call to itself.
///     - We first fetch the fields needed to satisfy some resolvable `@key` of the
///       entity type within this subgraph.
///     - Then, we use that to resolve the fields specified in the `@requires(fields:)`.
///     - This effectively splits the resolution within the subgraph: part comes from the
///       main query path, part comes via the internal entity resolution triggered by `@requires`.
///
/// The key fields need to be added to the output of the parent,
/// and input of the entity move.
///
/// The key is one of `node_index`'s, and its fields are read from `from`, the subgraph type the
/// objects are in. `None` when `from` can't resolve any of them.
#[instrument(level = "trace",skip_all, fields(
  node = graph.node(node_index).unwrap().to_string()
))]
fn find_satisfiable_key<'a>(
    graph: &'a Graph<'a>,
    fetch_graph: &mut FetchGraph<SingleTypeFetchStep>,
    supergraph: &'a SupergraphState,
    override_context: &'a PlannerOverrideContext,
    node_index: NodeIndex,
    from: NodeIndex,
    local_only: bool,
) -> Result<Option<&'a TypeAwareSelection<'a>>, FetchGraphError> {
    // This could be improved...
    // We added a flag to `can_satisfy_edge` and increased the complexity.

    let mut entity_moves_edges_to_self: Vec<EdgeReference<'a>> = graph
        .edges_from(node_index)
        .filter(|edge_reference| {
            let edge = graph.edge(edge_reference.id()).unwrap();
            matches!(edge, Edge::EntityMove(_))
                && (!local_only || {
                    let source = graph.node(node_index).unwrap();
                    let target = graph.node(edge_reference.target()).unwrap();
                    source.name_str() == target.name_str() && source.graph_id() == target.graph_id()
                })
        })
        .collect();
    entity_moves_edges_to_self.sort_by_key(|edge| std::cmp::Reverse(edge.weight().cost()));

    for edge_ref in entity_moves_edges_to_self {
        if satisfiable_from(
            graph,
            fetch_graph,
            supergraph,
            override_context,
            &edge_ref,
            from,
        )? {
            return edge_ref.weight().requirements().map(Some).ok_or_else(|| {
                FetchGraphError::Internal(String::from("Resolved empty Satisfiable Key"))
            });
        }
    }

    Ok(None)
}

/// Whether the requirements of `edge_ref` resolve right where `from` is, without leaving its
/// subgraph. The same few keys get asked about for every `@requires` field, so the answer is kept.
fn satisfiable_from(
    graph: &Graph<'_>,
    fetch_graph: &mut FetchGraph<SingleTypeFetchStep>,
    supergraph: &SupergraphState,
    override_context: &PlannerOverrideContext,
    edge_ref: &EdgeReference<'_>,
    from: NodeIndex,
) -> Result<bool, FetchGraphError> {
    if let Some(&satisfiable) = fetch_graph.satisfiable.get(&(edge_ref.id(), from)) {
        return Ok(satisfiable);
    }
    let satisfiable = can_satisfy_edge(
        graph,
        supergraph,
        override_context,
        edge_ref,
        &OperationPath {
            root_node: from,
            last_segment: None,
            cost: 0,
            union_context: None,
        },
        &Default::default(),
        true,
        // It's safe to use a noop CancellationToken here,
        // as the result of this function is guaranteed to be successful,
        // and fast.
        &CancellationToken::new(),
    )?
    .is_some();
    fetch_graph
        .satisfiable
        .insert((edge_ref.id(), from), satisfiable);
    Ok(satisfiable)
}

fn no_satisfiable_key() -> FetchGraphError {
    FetchGraphError::Internal(String::from("Failed to find Satisfiable Key"))
}

/// The key the entity call `step` was made with, when it calls `node`.
fn key_of_entity_call<'a>(
    graph: &'a Graph<'a>,
    fetch_graph: &FetchGraph<SingleTypeFetchStep>,
    step: NodeIndex,
    node: NodeIndex,
) -> Result<Option<&'a TypeAwareSelection<'a>>, FetchGraphError> {
    // `entity_moves` only holds moves by their own tail, see `process_entity_move_edge`
    let Some(&edge_index) = fetch_graph.entity_moves.get(&(step, node)) else {
        return Ok(None);
    };
    Ok(graph.edge(edge_index)?.requirements())
}

// TODO: simplfy args
#[allow(clippy::too_many_arguments)]
fn process_query_node(
    graph: &Graph<'_>,
    fetch_graph: &mut FetchGraph<SingleTypeFetchStep>,
    supergraph: &SupergraphState,
    override_context: &PlannerOverrideContext,
    query_node: &QueryTreeNode,
    parent_fetch_step_index: Option<NodeIndex>,
    response_path: &MergePath,
    fetch_path: &MergePath,
    requiring_fetch_step_index: Option<NodeIndex>,
    conditions: Conditions<'_>,
    created_from_requires: bool,
) -> Result<Vec<NodeIndex>, FetchGraphError> {
    // a condition of its own goes on top of the ones around it
    let with_own;
    let conditions = match &query_node.condition {
        Some(own) if conditions.from_root.last() != Some(own) => {
            with_own = [conditions.from_root, std::slice::from_ref(own)].concat();
            Conditions {
                from_root: &with_own,
                current: Some(own),
            }
        }
        Some(own) => Conditions {
            current: Some(own),
            ..conditions
        },
        None => conditions,
    };
    if let Some(edge_index) = query_node.edge_from_parent {
        let parent_fetch_step_index = parent_fetch_step_index.ok_or(FetchGraphError::IndexNone)?;
        let edge = graph.edge(edge_index)?;

        if matches!(edge, Edge::FieldMove(_) | Edge::ReentryMove(_)) {
            for requirement in query_node.context_requirements() {
                fetch_graph.add_output_typename(
                    parent_fetch_step_index,
                    response_path,
                    fetch_path,
                )?;
                let producers = process_query_node(
                    graph,
                    fetch_graph,
                    supergraph,
                    override_context,
                    requirement,
                    Some(parent_fetch_step_index),
                    response_path,
                    fetch_path,
                    None,
                    conditions,
                    true,
                )?;
                let coordinate = requirement.context_coordinate().ok_or_else(|| {
                    FetchGraphError::Internal("Missing context argument coordinate".to_string())
                })?;
                fetch_graph
                    .context_producers
                    .entry((response_path.clone(), coordinate))
                    .or_default()
                    .extend(producers);
            }
        }

        match edge {
            Edge::SubgraphEntrypoint {
                name,
                operation_kind,
            } => {
                let tail_node_index = graph.get_edge_tail(&edge_index)?;
                let tail_node = graph.node(tail_node_index)?;
                let type_name = match tail_node {
                    Node::QueryRoot(t) => t,
                    Node::MutationRoot(t) => t,
                    Node::SubscriptionRoot(t) => t,
                    Node::SubgraphType(t) => &t.name,
                };

                process_subgraph_entrypoint_edge(
                    graph,
                    fetch_graph,
                    supergraph,
                    override_context,
                    query_node,
                    parent_fetch_step_index,
                    name,
                    type_name,
                    operation_kind.clone(),
                    conditions,
                    created_from_requires,
                )
            }
            Edge::EntityMove(_) => process_entity_move_edge(
                graph,
                fetch_graph,
                supergraph,
                override_context,
                query_node,
                parent_fetch_step_index,
                response_path,
                fetch_path,
                requiring_fetch_step_index,
                edge_index,
                conditions,
                created_from_requires,
            ),
            Edge::ReentryMove(field) => process_subgraph_reentry(
                graph,
                fetch_graph,
                supergraph,
                edge_index,
                override_context,
                query_node,
                parent_fetch_step_index,
                created_from_requires,
                field,
                response_path,
                fetch_path,
                requiring_fetch_step_index,
                conditions,
            ),
            Edge::FieldMove(field) => match !field.context_arguments.is_empty()
                || (field.requirements.is_some()
                    && !requirements_resolve_in_place(graph, query_node)?)
            {
                true => process_requires_field_edge(
                    graph,
                    fetch_graph,
                    supergraph,
                    override_context,
                    query_node,
                    parent_fetch_step_index,
                    response_path,
                    requiring_fetch_step_index,
                    field,
                    edge_index,
                    conditions,
                    created_from_requires,
                ),
                false => {
                    // Required fields we can get from here go into the same fetch as the field.
                    process_requirements_for_fetch_steps(
                        graph,
                        fetch_graph,
                        supergraph,
                        override_context,
                        query_node,
                        parent_fetch_step_index,
                        requiring_fetch_step_index,
                        conditions,
                        response_path,
                        fetch_path,
                    )?;
                    process_plain_field_edge(
                        graph,
                        fetch_graph,
                        supergraph,
                        override_context,
                        query_node,
                        parent_fetch_step_index,
                        requiring_fetch_step_index,
                        response_path,
                        fetch_path,
                        field,
                        conditions,
                        created_from_requires,
                    )
                }
            },
            Edge::Selfie(type_name) => process_selfie_edge(
                graph,
                fetch_graph,
                supergraph,
                override_context,
                query_node,
                parent_fetch_step_index,
                requiring_fetch_step_index,
                response_path,
                fetch_path,
                type_name,
                conditions,
                created_from_requires,
            ),
            Edge::AbstractMove(type_name) => process_abstract_edge(
                graph,
                fetch_graph,
                supergraph,
                override_context,
                query_node,
                parent_fetch_step_index,
                requiring_fetch_step_index,
                response_path,
                fetch_path,
                type_name,
                conditions,
                created_from_requires,
            ),
            Edge::InterfaceObjectTypeMove(InterfaceObjectTypeMove {
                object_type_name, ..
            }) => process_interface_object_type_move_edge(
                graph,
                fetch_graph,
                supergraph,
                override_context,
                query_node,
                parent_fetch_step_index,
                response_path,
                fetch_path,
                requiring_fetch_step_index,
                edge_index,
                object_type_name,
                conditions,
                created_from_requires,
            ),
        }
    } else {
        process_noop_edge(
            graph,
            fetch_graph,
            supergraph,
            override_context,
            query_node,
            parent_fetch_step_index,
            response_path,
            fetch_path,
            requiring_fetch_step_index,
            conditions,
            created_from_requires,
        )
    }
}

/// Can we fetch the required fields right here, in the same fetch as the field?
///
/// Yes, when every path to them stays in the current subgraph. Required fields are
/// `@external`, so this only happens when a `@provides` gave us those fields at this spot.
fn requirements_resolve_in_place(
    graph: &Graph<'_>,
    query_node: &QueryTreeNode,
) -> Result<bool, FetchGraphError> {
    fn stays_in_subgraph(graph: &Graph<'_>, node: &QueryTreeNode) -> Result<bool, FetchGraphError> {
        if !node.requirements.is_empty() || !node.resolved_context_requirements().is_empty() {
            return Ok(false);
        }
        if let Some(edge_index) = node.edge_from_parent {
            if !matches!(
                graph.edge(edge_index)?,
                Edge::FieldMove(_) | Edge::AbstractMove(_) | Edge::Selfie(_)
            ) {
                return Ok(false);
            }
        }
        for child in node.children.iter() {
            if !stays_in_subgraph(graph, child)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    for requirement in query_node.requirements.iter() {
        if !stays_in_subgraph(graph, requirement)? {
            return Ok(false);
        }
    }
    Ok(true)
}

/// `path`, then the field `field` and a list segment per level of lists it returns. One
/// allocation, where pushing them one by one copies the path each time.
fn path_into_field(path: &MergePath, field: Segment, list_depth: usize) -> MergePath {
    MergePath {
        inner: path
            .inner
            .iter()
            .cloned()
            .chain(std::iter::once(field))
            .chain(std::iter::repeat_n(Segment::List, list_depth))
            .collect(),
    }
}

pub fn find_graph_roots<State>(graph: &FetchGraph<State>) -> Vec<NodeIndex> {
    let mut roots = Vec::new();

    // Iterate over all nodes in the graph
    for node_idx in graph.step_indices() {
        // Check if the node has any incoming edges.
        // The `next().is_none()` checks if the iterator is empty - no incoming edges.
        if graph.parents_of(node_idx).next().is_none() {
            roots.push(node_idx);
        }
    }

    roots
}

#[instrument(level = "trace", skip(graph, query_tree, supergraph, override_context), fields(
    requirements_count = query_tree.root.requirements.len(),
    children_count = query_tree.root.children.len(),
))]
pub fn build_fetch_graph_from_query_tree(
    graph: &Graph<'_>,
    supergraph: &SupergraphState,
    override_context: &PlannerOverrideContext,
    query_tree: QueryTree,
    operation: &OperationDefinition,
    options: &QueryPlannerOptions,
    cancellation_token: &CancellationToken,
) -> Result<FetchGraph<MultiTypeFetchStep>, FetchGraphError> {
    let mut fetch_graph = FetchGraph::new(
        operation
            .operation_kind
            .clone()
            .unwrap_or(OperationKind::Query),
    );
    fetch_graph.response_keys = ResponseKeys::from_operation(&operation.selection_set);
    // only names that look like ours can clash, so for nearly every operation this stays
    // empty and allocates nothing
    fetch_graph.operation_variables = operation
        .variable_definitions
        .iter()
        .flatten()
        .filter(|variable| variable.name.starts_with("__context_"))
        .map(|variable| variable.name.clone())
        .collect();

    process_query_node(
        graph,
        &mut fetch_graph,
        supergraph,
        override_context,
        &query_tree.root,
        None,
        &MergePath::default(),
        &MergePath::default(),
        None,
        Conditions::default(),
        false,
    )?;

    trace!("Done");

    let root_indexes = find_graph_roots(&fetch_graph);

    trace!("found roots");

    if root_indexes.is_empty() {
        return Err(FetchGraphError::NonSingleRootStep(0));
    }

    if root_indexes.len() > 1 {
        return Err(FetchGraphError::NonSingleRootStep(root_indexes.len()));
    }

    trace!("fetch graph before optimizations:");
    trace!("{}", fetch_graph);

    // fine to unwrap as we have already checked the length
    fetch_graph.root_index = Some(*root_indexes.first().unwrap());
    let batched = std::mem::take(&mut fetch_graph.batched);
    let mut fetch_graph = fetch_graph.to_multi_type();
    fetch_graph.finish_batched_steps(batched, supergraph)?;
    fetch_graph.optimize(supergraph, options, cancellation_token)?;
    fetch_graph.collect_variable_usages()?;

    trace!("fetch graph after optimizations:");
    trace!("{}", fetch_graph);

    Ok(fetch_graph)
}

#[cfg(test)]
mod tests {
    #[test]
    fn planned_requires_with_other_arguments_is_another_entry() {
        use crate::query_planner::ast::value::Value;
        use std::collections::HashMap;
        use std::hash::{BuildHasherDefault, Hasher};

        // every key gets the same hash, so only equality can tell them apart
        #[derive(Default)]
        struct SameHash;
        impl Hasher for SameHash {
            fn finish(&self) -> u64 {
                0
            }
            fn write(&mut self, _: &[u8]) {}
        }

        let planned = |limit: i64| super::PlannedRequires {
            edge: petgraph::graph::EdgeIndex::new(0),
            response_path: super::MergePath::default(),
            response_key: None,
            arguments: Some(super::ArgumentsMap::from((
                "limit".to_string(),
                Value::Int(limit),
            ))),
            conditions: Vec::new(),
            current_condition: None,
        };
        let mut steps: HashMap<_, _, BuildHasherDefault<SameHash>> = HashMap::default();
        steps.insert(planned(1), 1);
        assert_eq!(steps.get(&planned(1)), Some(&1));
        assert_eq!(steps.get(&planned(2)), None);
    }

    use super::*;

    /// A step added after a removal still gets an id no other step has.
    #[test]
    fn fetch_ids_are_not_reused_after_a_removal() {
        let mut fetch_graph = FetchGraph::new(OperationKind::Query);
        let first = create_noop_fetch_step(&mut fetch_graph, false);
        let removed = create_noop_fetch_step(&mut fetch_graph, false);
        let last = create_noop_fetch_step(&mut fetch_graph, false);
        fetch_graph.remove_step(removed);
        let added = create_noop_fetch_step(&mut fetch_graph, false);
        let id = |index| fetch_graph.get_step_data(index).unwrap().id.0;
        assert_eq!([id(first), id(last), id(added)], [1, 3, 4]);
    }

    #[test]
    fn same_entity_calls_are_children_of_one_parent_newest_first() {
        let mut fetch_graph = FetchGraph::new(OperationKind::Query);
        let users = SubgraphName("users".to_string());
        let reviews = SubgraphName("reviews".to_string());
        let path = MergePath::default();

        let parent = create_noop_fetch_step(&mut fetch_graph, false);
        let other_parent = create_noop_fetch_step(&mut fetch_graph, false);
        let mut add_child = |parent: NodeIndex, subgraph_name: &SubgraphName| {
            let index = create_fetch_step_for_entity_call(
                &mut fetch_graph,
                parent,
                subgraph_name,
                "User",
                "User",
                &path,
                Conditions::default(),
                false,
            );
            fetch_graph.connect(parent, index);
            index
        };
        let older = add_child(parent, &users);
        add_child(parent, &reviews);
        let newer = add_child(parent, &users);
        let under_other_parent = add_child(other_parent, &users);

        let found = |fetch_graph: &FetchGraph<SingleTypeFetchStep>, parent: NodeIndex| {
            fetch_graph
                .same_entity_calls(parent, &users, "User", &path, None)
                .collect::<Vec<_>>()
        };
        assert_eq!(found(&fetch_graph, parent), vec![newer, older]);
        assert_eq!(found(&fetch_graph, other_parent), vec![under_other_parent]);

        // connecting again keeps the one edge and doesn't index the child twice
        let edge = fetch_graph.connect(parent, older);
        assert_eq!(fetch_graph.connect(parent, older), edge);
        assert_eq!(
            fetch_graph
                .children_of(parent)
                .filter(|edge| edge.target() == older)
                .count(),
            1
        );
        assert_eq!(found(&fetch_graph, parent), vec![newer, older]);
    }
}
