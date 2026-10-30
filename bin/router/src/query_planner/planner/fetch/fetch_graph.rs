use crate::query_planner::ast::merge_path::{Condition, FieldPathSegment, MergePath, Segment};
use crate::query_planner::ast::operation::{OperationDefinition, VariableDefinition};
use crate::query_planner::ast::selection_item::SelectionItem;
use crate::query_planner::ast::selection_set::{
    FieldSelection, InlineFragmentSelection, SelectionSet,
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
use crate::query_planner::planner::fetch::response_keys::{place_of, ResponseKeys};
use crate::query_planner::planner::fetch::selections::FetchStepSelections;
use crate::query_planner::planner::fetch::state::{MultiTypeFetchStep, SingleTypeFetchStep};
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
use petgraph::visit::{Bfs, IntoNodeReferences};
use petgraph::Directed;
use petgraph::Direction;
use std::collections::{BTreeSet, HashSet, VecDeque};
use std::fmt::{Debug, Display};
use std::rc::Rc;
use tracing::{instrument, trace};

use super::error::FetchGraphError;

#[derive(Debug, Clone)]
pub struct FetchGraph<State> {
    pub(crate) graph: StableDiGraph<FetchStepData<State>, ()>,
    pub root_index: Option<NodeIndex>,
    pub(crate) operation_kind: OperationKind,
    /// The step whose output an entity call gets its representations from, so the step that
    /// holds the object the entity call extends. Only kept while the graph is built: merges
    /// would make the indexes stale, so `to_multi_type` drops it.
    pub(crate) entity_owners: HashMap<NodeIndex, NodeIndex>,
    /// The keys of the fields the planner adds itself. Only needed while the graph is built.
    pub(crate) response_keys: ResponseKeys,
    pub(crate) context_producers: HashMap<(MergePath, (EdgeIndex, usize)), Vec<NodeIndex>>,
    pub(crate) context_variable_counter: usize,
    pub(crate) operation_variables: HashSet<String>,
    /// Each step's children by the entity call they make, see `entity_call_hash`, newest last. So
    /// finding a step to reuse doesn't go through all the children, thousands on a big query.
    /// Only kept while the graph is built: until the merges, a step keeps making the same entity
    /// call and no edge goes away.
    pub(crate) children_by_entity_call: Option<HashMap<(NodeIndex, u64), Vec<NodeIndex>>>,
    /// The conditions of the query nodes being processed, from the root down. Only needed while
    /// the graph is built.
    pub(crate) condition_chain: Vec<Condition>,
}

impl FetchGraph<SingleTypeFetchStep> {
    pub fn to_multi_type(self) -> FetchGraph<MultiTypeFetchStep> {
        let new_graph = self
            .graph
            .map(|_, w| w.clone().into_multi_type(), |_, _| ());

        FetchGraph {
            graph: new_graph,
            root_index: self.root_index,
            operation_kind: self.operation_kind,
            entity_owners: HashMap::new(),
            response_keys: ResponseKeys::default(),
            context_producers: HashMap::new(),
            context_variable_counter: 0,
            operation_variables: HashSet::new(),
            children_by_entity_call: None,
            condition_chain: Vec::new(),
        }
    }

    pub fn new(kind: OperationKind) -> Self {
        Self {
            graph: StableDiGraph::new(),
            root_index: None,
            operation_kind: kind,
            entity_owners: HashMap::new(),
            response_keys: ResponseKeys::default(),
            context_producers: HashMap::new(),
            context_variable_counter: 0,
            operation_variables: HashSet::new(),
            children_by_entity_call: Some(HashMap::new()),
            condition_chain: Vec::new(),
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
        let entity_call =
            entity_call_hash(subgraph_name, input_type_name, response_path, condition);
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
                    self.get_step_data(child).ok().and_then(entity_call_hash_of),
                    Some(entity_call),
                    "indexed step [{}] changed what it's reused on",
                    child.index(),
                );
            })
    }
}

impl FetchGraph<SingleTypeFetchStep> {
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
                .input(&place_of(response_path), &selection.selection_set),
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
        let key = self
            .response_keys
            .wire_key(&place_of(response_path), &field);
        (key != field.name).then_some(key)
    }
}

impl<State> FetchGraph<State> {
    pub fn all_nodes(&self) -> NodeReferences<'_, FetchStepData<State>> {
        self.graph.node_references()
    }

    pub fn create_fetch_id(&self) -> i64 {
        (self.graph.node_count() + 1) as i64
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
        if let (Some(index), Some(entity_call)) = (
            self.children_by_entity_call.as_mut(),
            self.graph
                .node_weight(child_index)
                .and_then(entity_call_hash_of),
        ) {
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

fn create_noop_fetch_step(
    fetch_graph: &mut FetchGraph<SingleTypeFetchStep>,
    created_from_requires: bool,
) -> NodeIndex {
    let flags = if created_from_requires {
        FetchStepFlags::USED_FOR_REQUIRES
    } else {
        FetchStepFlags::empty()
    };

    fetch_graph.add_step(FetchStepData {
        id: fetch_graph.create_fetch_id(),
        service_name: SubgraphName::any(),
        response_path: MergePath::default(),
        input: FetchStepSelections::new_empty(),
        output: FetchStepSelections::new_empty(),
        flags,
        condition: None,
        planned_under: Vec::new(),
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
    condition: Option<&Condition>,
    used_for_requires: bool,
) -> NodeIndex {
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

    let step_index = fetch_graph.add_step(FetchStepData {
        id: fetch_graph.create_fetch_id(),
        service_name: subgraph_name.clone(),
        response_path: response_path.clone(),
        input,
        output: FetchStepSelections::new(output_type_name),
        flags,
        condition: condition.cloned(),
        planned_under: fetch_graph.condition_chain.clone(),
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
    condition: Option<&Condition>,
) -> NodeIndex {
    let idx = fetch_graph.add_step(FetchStepData {
        id: fetch_graph.create_fetch_id(),
        service_name: subgraph_name.clone(),
        response_path: response_path.clone(),
        input: FetchStepSelections::new(type_name),
        output: FetchStepSelections::new(type_name),
        flags: FetchStepFlags::empty(),
        condition: condition.cloned(),
        planned_under: fetch_graph.condition_chain.clone(),
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
/// response path and condition.
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
    response_path.hash(&mut hasher);
    hasher.finish()
}

/// `entity_call_hash` of a step already in the graph.
fn entity_call_hash_of<State>(step: &FetchStepData<State>) -> Option<u64> {
    let input_type_name = step.input.first_definition_name()?;
    Some(entity_call_hash(
        &step.service_name,
        input_type_name,
        &step.response_path,
        step.condition.as_ref(),
    ))
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
    requires: Option<&TypeAwareSelection<'_>>,
    condition: Option<&Condition>,
    created_from_requires: bool,
) -> Result<NodeIndex, FetchGraphError> {
    let matching_child_index = if requires.is_some() {
        None
    } else {
        fetch_graph
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
                        && key.is_none_or(|key| {
                            step.input.definition_name() == key.type_name
                                && step.input.selection_set().contains(&key.selection_set)
                        })
                        // steps with requirements aren't reused here, the optimizations
                        // try that later
                        && !step.flags.contains(FetchStepFlags::USED_FOR_REQUIRES)
                })
            })
    };

    match matching_child_index {
        Some(idx) => {
            trace!(
                "found existing fetch step [{}] for entity move requirement({}) key({}) in children of {}",
                idx.index(),
                requires.map(|r| r.to_string()).unwrap_or_default(),
                key.map(|r| r.to_string()).unwrap_or_default(),
                parent_fetch_step_index.index(),
            );
            let chain = &fetch_graph.condition_chain;
            let step = fetch_graph
                .graph
                .node_weight_mut(idx)
                .ok_or(FetchGraphError::IndexNone)?;
            step.planned_under
                .retain(|condition| chain.contains(condition));
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
                condition,
                created_from_requires || requires.is_some(),
            );
            if let Some(selection) = key {
                let step = fetch_graph.get_step_data_mut(step_index)?;
                step.input.add(&selection.selection_set)?
            }

            trace!(
                "created a new fetch step [{}] subgraph({}) type({}) requirement({}) key({}) in children of {}",
                step_index.index(),
                subgraph_name,
                input_type_name,
                requires.map(|r| r.to_string()).unwrap_or_default(),
                key.map(|r| r.to_string()).unwrap_or_default(),
                parent_fetch_step_index.index(),
            );

            Ok(step_index)
        }
    }
}

fn ensure_fetch_step_for_requirement(
    fetch_graph: &mut FetchGraph<SingleTypeFetchStep>,
    parent_fetch_step_index: NodeIndex,
    subgraph_name: &SubgraphName,
    type_name: &str,
    response_path: &MergePath,
    condition: Option<&Condition>,
    requirement: &TypeAwareSelection<'_>,
) -> Result<NodeIndex, FetchGraphError> {
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
                condition,
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
    condition: Option<&Condition>,
    created_from_requires: bool,
) -> Result<Vec<NodeIndex>, FetchGraphError> {
    if query_node.children.is_empty() {
        return Ok(vec![parent_fetch_step_index]);
    }

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
            condition,
            created_from_requires,
        )?);
    }

    Ok(leaf_fetch_step_indexes)
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
    condition: Option<&Condition>,
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
            condition,
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
    condition: Option<&Condition>,
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
        condition,
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
    condition: Option<&Condition>,
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

    // A key of an interface, like `Animal @key(fields: "id")`, sends every animal. When all that's
    // asked of them is in `... on Dog` and `... on Cat`, a Bird would go with nothing to ask of
    // it, so the input only takes the types that are asked for. Not from an
    // `@interfaceObject` though, its data has the interface's name in `__typename`.
    let from_interface_object = matches!(
        graph.node(graph.get_edge_head(&edge_index)?)?,
        Node::SubgraphType(t) if t.is_interface_object
    );
    let asked_types = only_asked_types(graph, query_node)?;
    let narrowed = !from_interface_object && !asked_types.is_empty();
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

    let head_node_index = graph.get_edge_head(&edge_index)?;
    let head_node = graph.node(head_node_index)?;
    let input_type_name = match head_node {
        Node::SubgraphType(t) => &t.name,
        _ => return Err(FetchGraphError::ExpectedSubgraphType),
    };

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
        None,
        condition,
        created_from_requires,
    )?;

    let fetch_step = fetch_graph.get_step_data_mut(fetch_step_index)?;
    trace!(
        "adding input requirement '{}' to fetch step [{}]",
        requirement,
        fetch_step_index.index()
    );
    fetch_step.input.add(&requirement.selection_set)?;

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

    let parent_fetch_step = fetch_graph.get_step_data_mut(parent_fetch_step_index)?;
    parent_fetch_step
        .output
        .add_selection_typename(fetch_path)?;

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
        condition,
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
        condition,
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
    condition: Option<&Condition>,
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
        None,
        condition,
        created_from_requires,
    )?;

    trace!(
        "adding input requirement '{}' to fetch step [{}]",
        requirement,
        step_for_children_index.index()
    );
    let key_to_reenter_subgraph = find_satisfiable_key(
        graph,
        supergraph,
        override_context,
        query_node.requirements.first().unwrap().node_index,
        false,
    )?;
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

    let parent_fetch_step = fetch_graph.get_step_data_mut(parent_fetch_step_index)?;
    parent_fetch_step
        .output
        .add_selection_typename(fetch_path)?;

    // In all cases it's `__typename` that needs to be resolved by another subgraph.
    trace!("Creating a fetch step for requirement of @interfaceObject");
    let step_for_requirements_index = create_fetch_step_for_entity_call(
        fetch_graph,
        parent_fetch_step_index,
        head_subgraph_name,
        object_type_name,
        interface_type_name,
        response_path,
        condition,
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
        condition,
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
        condition,
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
        None,
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
        None,
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
    condition: Option<&Condition>,
) -> Result<Vec<NodeIndex>, FetchGraphError> {
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
    let condition_in_path = if let Some(condition) = condition {
        fetch_path.inner.iter().any(|segment| {
            matches!(
                segment,
                Segment::TypeCondition(_, Some(c)) | Segment::Field(_, _, Some(c)) if c == condition
            )
        })
    } else {
        false
    };

    let condition_on_step = condition.is_some_and(|c| {
        fetch_graph.is_condition_on_step(parent_fetch_step_index, requiring_fetch_step_index, c)
    });

    let should_strip_condition = condition_in_path || condition_on_step;
    let field_alias = fetch_graph.alias_at(
        response_path,
        query_node,
        &reentry_move.name,
        created_from_requires,
    );
    let parent_fetch_step = fetch_graph.get_step_data_mut(parent_fetch_step_index)?;
    parent_fetch_step.output.add_at_path(
        fetch_path,
        SelectionSet {
            items: vec![SelectionItem::Field(FieldSelection {
                name: reentry_move.name.to_string(),
                alias: field_alias.clone(),
                selections: SelectionSet {
                    items: vec![SelectionItem::Field(FieldSelection::new_typename())],
                },
                arguments: query_node.selection_arguments().cloned(),
                skip_if: condition.and_then(|c| {
                    if should_strip_condition {
                        None
                    } else {
                        c.to_skip_if()
                    }
                }),
                include_if: condition.and_then(|c| {
                    if should_strip_condition {
                        None
                    } else {
                        c.to_include_if()
                    }
                }),
                omit_from_response: false,
            })],
        },
    )?;

    let segment_field = Rc::new(FieldPathSegment::new(
        reentry_move.name.to_string(),
        field_alias,
    ));
    let mut child_response_path = response_path.push(Segment::Field(segment_field, 0, None));
    for _ in 0..reentry_move.list_depth {
        child_response_path = child_response_path.push(Segment::List);
    }

    let fetch_step_index = create_fetch_step_for_root_move(
        fetch_graph,
        parent_fetch_step_index,
        subgraph_name,
        type_name,
        OperationKind::Query,
        query_node.mutation_field_position,
        &child_response_path,
        condition,
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
        None,
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
    condition: Option<&Condition>,
    created_from_requires: bool,
) -> Result<Vec<NodeIndex>, FetchGraphError> {
    let condition_on_step = condition.is_some_and(|c| {
        fetch_graph.is_condition_on_step(parent_fetch_step_index, requiring_fetch_step_index, c)
    });

    let parent_fetch_step = fetch_graph.get_step_data_mut(parent_fetch_step_index)?;
    trace!(
        "starting an inline fragment for type '{}' to fetch step [{}]",
        parent_fetch_step_index.index(),
        target_type_name,
    );
    parent_fetch_step.output.add_at_path(
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
        condition,
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
    condition: Option<&Condition>,
    created_from_requires: bool,
) -> Result<Vec<NodeIndex>, FetchGraphError> {
    // The fragment's condition goes down to its children, and each one applies it. A child with
    // a condition of its own only carries that one though, so in
    // `... on Dog @include(if: $x) { name @skip(if: $y) }` `name` would lose `$x`. Then the
    // fragment keeps the condition, like a field does.
    let child_has_other_condition = condition.is_some_and(|condition| {
        query_node
            .children
            .iter()
            .any(|child| child.condition.as_ref().is_some_and(|c| c != condition))
    });
    let fragment_condition = condition.filter(|_| child_has_other_condition);
    let condition_in_path = fragment_condition.is_some_and(|condition| {
        fetch_path.inner.iter().any(|segment| {
            matches!(
                segment,
                Segment::TypeCondition(_, Some(c)) | Segment::Field(_, _, Some(c)) if c == condition
            )
        })
    });
    let condition_on_step = fragment_condition.is_some_and(|c| {
        fetch_graph.is_condition_on_step(parent_fetch_step_index, requiring_fetch_step_index, c)
    });
    let segment_condition = if condition_in_path || condition_on_step {
        None
    } else {
        fragment_condition.cloned()
    };

    let parent_fetch_step = fetch_graph.get_step_data_mut(parent_fetch_step_index)?;
    trace!(
        "adding output field '__typename' and starting an inline fragment for type '{}' to fetch step [{}]",
        parent_fetch_step_index.index(),
        target_type_name,
    );

    parent_fetch_step.output.add_at_path(
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
        condition,
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
    condition: Option<&Condition>,
    created_from_requires: bool,
) -> Result<Vec<NodeIndex>, FetchGraphError> {
    if let Some(requiring_fetch_step_index) = requiring_fetch_step_index {
        trace!(
            "connecting parent fetch step [{}] to requiring fetch step [{}]",
            parent_fetch_step_index.index(),
            requiring_fetch_step_index.index()
        );
        fetch_graph.connect(parent_fetch_step_index, requiring_fetch_step_index);
    }

    // If we are inserting into "... | [Product] @include(if: $bool)", we don't need the condition on the field.
    let condition_in_path = if let Some(condition) = condition {
        fetch_path.inner.iter().any(|segment| {
            matches!(
                segment,
                Segment::TypeCondition(_, Some(c)) | Segment::Field(_, _, Some(c)) if c == condition
            )
        })
    } else {
        false
    };

    let condition_on_step = condition.is_some_and(|c| {
        fetch_graph.is_condition_on_step(parent_fetch_step_index, requiring_fetch_step_index, c)
    });

    let should_strip_condition = condition_in_path || condition_on_step;

    let field_alias = fetch_graph.alias_at(
        response_path,
        query_node,
        field_move.name,
        created_from_requires,
    );
    let parent_fetch_step = fetch_graph.get_step_data_mut(parent_fetch_step_index)?;
    trace!(
        "adding output field '{}' to fetch step [{}]",
        field_move.name,
        parent_fetch_step_index.index()
    );

    parent_fetch_step.output.add_at_path(
        fetch_path,
        SelectionSet {
            items: vec![SelectionItem::Field(FieldSelection {
                name: field_move.name.to_string(),
                alias: field_alias.clone(),
                selections: SelectionSet::default(),
                arguments: query_node.selection_arguments().cloned(),
                skip_if: condition.and_then(|c| {
                    if should_strip_condition {
                        None
                    } else {
                        c.to_skip_if()
                    }
                }),
                include_if: condition.and_then(|c| {
                    if should_strip_condition {
                        None
                    } else {
                        c.to_include_if()
                    }
                }),
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
    let segment_condition = if should_strip_condition {
        None
    } else {
        condition.cloned()
    };
    let mut child_response_path = response_path.push(Segment::Field(
        segment_field.clone(),
        segment_args_hash,
        segment_condition.clone(),
    ));
    let mut child_fetch_path = fetch_path.push(Segment::Field(
        segment_field,
        segment_args_hash,
        segment_condition,
    ));

    for _ in 0..field_move.list_depth {
        child_response_path = child_response_path.push(Segment::List);
        child_fetch_path = child_fetch_path.push(Segment::List);
    }

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
        condition,
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
    condition: Option<&Condition>,
    created_from_requires: bool,
) -> Result<Vec<NodeIndex>, FetchGraphError> {
    let requires = field_move.requirements.as_ref();

    let head_node_index = graph.get_edge_head(&edge_index)?;
    let head_node = graph.node(head_node_index)?;
    let (head_type_name, head_subgraph_name, is_interface_object) = match head_node {
        Node::SubgraphType(t) => (t.name, &t.subgraph, &t.is_interface_object),
        _ => return Err(FetchGraphError::ExpectedSubgraphType),
    };

    let key_to_reenter_subgraph = find_satisfiable_key(
        graph,
        supergraph,
        override_context,
        head_node_index,
        !field_move.context_arguments.is_empty(),
    )?;
    // What the entity calls read, and the keys the step holding the objects writes.
    let requires_input = requires.map(|requires| fetch_graph.read_at(response_path, requires));
    let key_input = fetch_graph.read_at(response_path, key_to_reenter_subgraph);
    let key_output = fetch_graph.response_keys.output(
        &place_of(response_path),
        &key_to_reenter_subgraph.selection_set,
    );
    let field_alias = fetch_graph.alias_at(
        response_path,
        query_node,
        field_move.name,
        created_from_requires,
    );

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
            .ok_or(FetchGraphError::MissingEntityOwner(
                parent_fetch_step_index.index(),
            ))?,
    };

    // When a field (foo) is annotated with `@requires(fields: "bar")`
    // We want to create new FetchStep (entity move) for that field (foo)
    // or reuse an existing one if the requirement matches
    // The new FetchStep will have `foo` as the output and `bar` as the input.
    // The `bar` should be fetched by one of the parents.
    trace!("Creating a fetch step for children of @requires");
    let step_for_children_index = if field_move.context_arguments.is_empty() {
        ensure_fetch_step_for_requirement(
            fetch_graph,
            real_parent_fetch_step_index,
            head_subgraph_name,
            head_type_name,
            response_path,
            condition,
            requires_input
                .as_ref()
                .ok_or(FetchGraphError::MissingRequires)?,
        )?
    } else {
        create_fetch_step_for_entity_call(
            fetch_graph,
            real_parent_fetch_step_index,
            head_subgraph_name,
            head_type_name,
            head_type_name,
            response_path,
            condition,
            true,
        )
    };

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
            fetch_graph.connect(producer, step_for_children_index);
            fetch_graph
                .get_step_data_mut(step_for_children_index)?
                .context_producers
                .insert(id);
        }
        let variable = loop {
            let name = format!("__context_{}", fetch_graph.context_variable_counter);
            fetch_graph.context_variable_counter += 1;
            if !fetch_graph.operation_variables.contains(&name) {
                break name;
            }
        };
        arguments.add_argument(argument.name.clone(), Value::Variable(variable.clone()));
        let selection = &argument.selections[requirement.selection];
        let output = fetch_graph
            .response_keys
            .output(&place_of(&ancestor_path), &selection.selection_set);
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
        let step = fetch_graph.get_step_data_mut(step_for_children_index)?;
        step.context_rewrites
            .get_or_insert_default()
            .extend(rewrites);
        step.context_variable_definitions.push(VariableDefinition {
            name: variable,
            variable_type: argument.type_in_graph.clone(),
            default_value: None,
        });
    }
    let arguments = (!arguments.is_empty()).then_some(arguments);
    let fetch_arguments_hash = arguments
        .as_ref()
        .map(|arguments| arguments.hash_u64())
        .unwrap_or(0);
    let step_for_children = fetch_graph.get_step_data_mut(step_for_children_index)?;

    step_for_children.output.add_at_path(
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
            step_for_children_index.index(),
        );
        step_for_children.add_input_rewrite(FetchRewrite::ValueSetter(ValueSetter {
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
            step_for_children_index.index()
        );
        step_for_children.input.add(&requires_input.selection_set)?;
    }
    trace!(
        "Adding {} to fetch([{}]).input (key re-enter)",
        key_input,
        step_for_children_index.index()
    );
    step_for_children.input.add(&key_input.selection_set)?;

    let real_parent_fetch_step = fetch_graph.get_step_data_mut(real_parent_fetch_step_index)?;

    let key_to_reenter_at = if real_parent_fetch_step.response_path.len() > response_path.len() {
        return Err(FetchGraphError::Internal(
            "Response path is longer than expected".to_string(),
        ));
    } else {
        response_path.slice_from(real_parent_fetch_step.response_path.len())
    };

    trace!(
        "Adding {} to fetch([{}]).output at path {}",
        key_to_reenter_subgraph,
        real_parent_fetch_step_index.index(),
        key_to_reenter_at
    );

    real_parent_fetch_step
        .output
        .add_at_path(&key_to_reenter_at, key_output)?;

    real_parent_fetch_step
        .output
        .add_selection_typename(&key_to_reenter_at)?;

    // On a `@provides` path, some required fields only exist on that path. The subgraph can't
    // return them from `_entities`, so the fetch that went through the path gets them, and
    // whatever needs another fetch starts from there too.
    let (step_for_requirements_index, requirements_fetch_path) = if requires.is_none()
        || head_node.is_using_provides()
    {
        (real_parent_fetch_step_index, key_to_reenter_at)
    } else {
        trace!("Creating a fetch step for requirement of @requires");
        let step_for_requirements_index = create_fetch_step_for_entity_call(
            fetch_graph,
            real_parent_fetch_step_index,
            head_subgraph_name,
            head_type_name,
            head_type_name,
            response_path,
            condition,
            true,
        );
        let step_for_requirements = fetch_graph.get_step_data_mut(step_for_requirements_index)?;
        trace!(
            "Adding {} to fetch([{}]).input",
            key_input,
            step_for_requirements_index.index()
        );
        step_for_requirements.input.add(&key_input.selection_set)?;
        fetch_graph.connect(real_parent_fetch_step_index, step_for_requirements_index);

        (step_for_requirements_index, MergePath::default())
    };

    let segment_field = Rc::new(FieldPathSegment::new(
        field_move.name.to_string(),
        field_alias,
    ));
    let mut child_response_path = response_path.push(Segment::Field(
        segment_field.clone(),
        fetch_arguments_hash,
        None,
    ));
    let mut child_fetch_path =
        MergePath::default().push(Segment::Field(segment_field, fetch_arguments_hash, None));

    for _ in 0..field_move.list_depth {
        child_response_path = child_response_path.push(Segment::List);
        child_fetch_path = child_fetch_path.push(Segment::List);
    }

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
            Some(step_for_requirements_index),
            response_path,
            &requirements_fetch_path,
            None,
            condition,
            true,
        )?
    } else {
        Vec::new()
    };

    //
    // Given `f0 { f1 @requires(fields: f2) }`
    // - step_for_requirements    -> f2
    // - step_for_children        -> f1
    // - parent                   -> f0
    //
    // parent -> step_for_requirements -> step_for_children
    //
    // f0 -> f2 -> f1
    //
    // in case of `f2 @requires(fields: f3)`:
    // f0 -> f3 -> f2 -> f1
    //
    // and so on.
    //
    // Basically any leaf becomes a parent of current `step_for_children`.
    // This way we wait for the entire chain of fetches to be resolved before we move to resolve a field with `@requires`.

    if leaf_fetch_step_indexes.is_empty() {
        trace!("Connecting fetch that pulls requirements with fetch that resolves the field with @requires");
        fetch_graph.connect(step_for_requirements_index, step_for_children_index);
    } else {
        trace!("Connecting leaf fetches of requirements to fetch with @requires");
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
        &child_response_path,
        &child_fetch_path,
        requiring_fetch_step_index,
        condition,
        created_from_requires,
    )
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
#[instrument(level = "trace",skip_all, fields(
  node = graph.node(node_index).unwrap().to_string()
))]
fn find_satisfiable_key<'a>(
    graph: &'a Graph<'a>,
    supergraph: &'a SupergraphState,
    override_context: &'a PlannerOverrideContext,
    node_index: NodeIndex,
    local_only: bool,
) -> Result<&'a TypeAwareSelection<'a>, FetchGraphError> {
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
        if can_satisfy_edge(
            graph,
            supergraph,
            override_context,
            &edge_ref,
            &OperationPath {
                root_node: node_index,
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
        .is_some()
        {
            return edge_ref
                .weight()
                .requirements()
                .ok_or(FetchGraphError::Internal(String::from(
                    "Resolved empty Satisfiable Key",
                )));
        }
    }

    Err(FetchGraphError::Internal(String::from(
        "Failed to find Satisfiable Key",
    )))
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
    condition: Option<&Condition>,
    created_from_requires: bool,
) -> Result<Vec<NodeIndex>, FetchGraphError> {
    let own = query_node
        .condition
        .as_ref()
        .filter(|own| fetch_graph.condition_chain.last() != Some(*own));
    if let Some(own) = own {
        fetch_graph.condition_chain.push(own.clone());
    }
    let result = process_query_node_edge(
        graph,
        fetch_graph,
        supergraph,
        override_context,
        query_node,
        parent_fetch_step_index,
        response_path,
        fetch_path,
        requiring_fetch_step_index,
        condition,
        created_from_requires,
    );
    if own.is_some() {
        fetch_graph.condition_chain.pop();
    }
    result
}

// TODO: simplfy args
#[allow(clippy::too_many_arguments)]
fn process_query_node_edge(
    graph: &Graph<'_>,
    fetch_graph: &mut FetchGraph<SingleTypeFetchStep>,
    supergraph: &SupergraphState,
    override_context: &PlannerOverrideContext,
    query_node: &QueryTreeNode,
    parent_fetch_step_index: Option<NodeIndex>,
    response_path: &MergePath,
    fetch_path: &MergePath,
    requiring_fetch_step_index: Option<NodeIndex>,
    condition: Option<&Condition>,
    created_from_requires: bool,
) -> Result<Vec<NodeIndex>, FetchGraphError> {
    let condition = query_node.condition.as_ref().or(condition);
    if let Some(edge_index) = query_node.edge_from_parent {
        let parent_fetch_step_index = parent_fetch_step_index.ok_or(FetchGraphError::IndexNone)?;
        let edge = graph.edge(edge_index)?;

        if matches!(edge, Edge::FieldMove(_) | Edge::ReentryMove(_)) {
            for requirement in query_node.context_requirements() {
                fetch_graph
                    .get_step_data_mut(parent_fetch_step_index)?
                    .output
                    .add_selection_typename(fetch_path)?;
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
                    condition,
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
                condition,
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
                condition,
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
                    condition,
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
                        condition,
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
                        condition,
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
                condition,
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
                condition,
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
                condition,
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
            condition,
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
        None,
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
    let mut fetch_graph = fetch_graph.to_multi_type();
    fetch_graph.optimize(supergraph, options, cancellation_token)?;
    fetch_graph.collect_variable_usages()?;

    trace!("fetch graph after optimizations:");
    trace!("{}", fetch_graph);

    Ok(fetch_graph)
}

#[cfg(test)]
mod tests {
    use super::*;

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
                None,
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
