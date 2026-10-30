//! This module rewrites the query plan to make execution faster.
//!
//! It merges multiple Flatten(Fetch) nodes to the same subgraph into one batched fetch.
//! This reduces the number of HTTP requests.
//!
//! If we have multiple entity fetches inside one `Parallel` block:
//! ```ignore
//! Parallel {
//!   Flatten(path: "a") { Fetch(service: "a") }
//!   Flatten(path: "b") { Fetch(service: "a") }
//!   Flatten(path: "c") { Fetch(service: "a") }
//! }
//! ```
//!
//! The optimization will merge them into:
//! ```ignore
//! Parallel {
//!   BatchFetch {
//!     entityBatch {
//!       aliases: [
//!         { alias: "_e0", paths: ["a", "b", "c"], ... }
//!       ]
//!     }
//!   }
//! }
//! ```
//!
//! We batch only compatible fetches:
//! - Fetches must target the same subgraph
//! - Variable type and default value must be compatible
//! - `input + output + rewrites` must match
//!
//! Each final shape group becomes one alias in `BatchFetchNode`.
//!
//! Here's an explanation of how the optimization works:
//! 4 fetches to same service
//! - F1: input: Product, selection: { name },   vars: `$locale: String`
//! - F2: input: Product, selection: { name },   vars: `$locale: String`
//! - F3: input: Product, selection: { price },  vars: `$locale: String`
//! - F4: input: User,    selection: { name },   vars: `$locale: String`
//!
//! Partitioning:
//! 1. By subgraph:     [F1, F2, F3, F4] (all same service)
//! 2. By variables:    [F1, F2, F3]     (compatible $locale),  [F4] (different input)
//! 3. By shape:        [F1, F2]         (same Product.name),   [F3] (different output)
//!
//! The outcome is a single http request with multiple entity aliases:
//!   BatchFetch
//!     e0: _entities (for F1 and F2)
//!     e1: _entities (for F3)
//!     e2: _entities (for F4)
//!
//! Fetches under an `Include` or `Skip` (nested ones too) can join a batch. Conditions aren't
//! part of the shape, each path of an alias gets the conditions of the fetches that reach it
//! instead, and paths under the same ones form a group. The router only collects from the groups
//! that hold, and an alias with none of them holding doesn't run. It tells the subgraph with a
//! `Boolean!` it sends per alias, `_e1: _entities(...) @include(if: $__batch_run_1)`, so aliases
//! under other conditions can still share their representations.
//!
//! Paths of one alias that only differ in the types of a type condition fold into one path.
//!

use ahash::{HashMap, HashMapExt, HashSet, HashSetExt, RandomState};
use indexmap::IndexMap;
use std::{
    collections::BTreeSet,
    hash::{Hash, Hasher},
};

use xxhash_rust::xxh3::Xxh3;

use crate::query_planner::{
    ast::requires::RequiresSelectionSet,
    ast::{
        hash::{ASTHash, SemanticShapeHashContext},
        minification::minify_operation,
        operation::{OperationDefinition, PlanningFetchOperation, VariableDefinition},
        selection_item::SelectionItem,
        selection_set::{FieldSelection, SelectionSet},
        value::Value,
    },
    planner::error::QueryPlanError,
    planner::plan_nodes::planning::{
        custom_scalar_paths_for_entities_selection, BatchFetchNode, ConditionGroup, ConditionNode,
        CustomScalarPaths, EntityBatch, EntityBatchAlias, FetchContext, FetchNode, FetchRewrite,
        FlattenNode, FlattenNodePath, MergePaths, PathGroup, PathSegment, PlanNode, RunIf,
        TypeCondition,
    },
    planner::QueryPlannerOptions,
    state::supergraph_state::{OperationKind, SupergraphState, TypeNode},
};

/// Fast key used before full shape comparison
///
/// Shape = `requires + entities_selection + input_rewrites + output_rewrites`.
///
/// We first compare hashes (fast), then compare full values (exact).
/// This keeps grouping fast and still safe with hash collisions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct ShapeKey {
    requires_hash: u64,
    entities_selection_hash: u64,
    input_rewrites_hash: u64,
    output_rewrites_hash: u64,
    context_rewrites_hash: u64,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct RepresentationsInputKey {
    requires: SelectionSet,
    input_rewrites: Option<Vec<FetchRewrite>>,
    context: Option<Box<FetchContext>>,
    merge_paths: Vec<FlattenNodePath>,
    /// `None` for an alias with one group, all of its paths run together. With more groups,
    /// some paths of an alias may run while others don't, so only aliases with the same
    /// groups collect the same representations.
    path_groups: Option<Vec<(RunIf, Vec<FlattenNodePath>)>>,
}

struct BatchFetchBuilder<'a> {
    merged_non_representation_variables: &'a [VariableDefinition],
    operation_variable_definitions: Vec<VariableDefinition>,
    operation_selection_items: Vec<SelectionItem>,
    batched_aliases: Vec<EntityBatchAlias>,
    used_variable_names: HashSet<String>,
    representations_var_index: usize,
    run_var_index: usize,
    variable_usages: BTreeSet<String>,
    representations_var_by_input_key: HashMap<RepresentationsInputKey, String>,
    custom_scalar_paths: CustomScalarPaths,
}

impl<'a> BatchFetchBuilder<'a> {
    fn new(
        merged_non_representation_variables: &'a [VariableDefinition],
        alias_count: usize,
    ) -> Self {
        Self {
            merged_non_representation_variables,
            operation_variable_definitions: Vec::with_capacity(
                alias_count + merged_non_representation_variables.len(),
            ),
            operation_selection_items: Vec::with_capacity(alias_count),
            batched_aliases: Vec::with_capacity(alias_count),
            used_variable_names: merged_non_representation_variables
                .iter()
                .map(|var| var.name.clone())
                .collect(),
            representations_var_index: 0,
            run_var_index: 0,
            variable_usages: BTreeSet::new(),
            representations_var_by_input_key: HashMap::new(),
            custom_scalar_paths: CustomScalarPaths::default(),
        }
    }

    fn add_shape_group(
        &mut self,
        alias_index: usize,
        shape_group: &[EntityFetch],
        supergraph: &SupergraphState,
    ) -> Result<(), QueryPlanError> {
        let representative = shape_group.first().ok_or_else(|| {
            QueryPlanError::Internal("Batched entities shape group cannot be empty".to_string())
        })?;

        for candidate in shape_group {
            if let Some(candidate_variable_usages) = &candidate.variable_usages {
                self.variable_usages
                    .extend(candidate_variable_usages.iter().cloned());
            }
        }

        let alias = format!("_e{alias_index}");
        let path_groups = Self::collect_path_groups(shape_group);
        let representations_variable_name =
            self.get_or_create_representations_var(representative, &path_groups);
        // The router works out the conditions, the subgraph only gets a yes or no.
        let can_be_off = path_groups.iter().all(|group| !group.run_if.is_always());
        let run_variable_name = can_be_off.then(|| {
            let name = next_unique_variable_name(
                "__batch_run_",
                &mut self.used_variable_names,
                &mut self.run_var_index,
            );
            self.operation_variable_definitions
                .push(VariableDefinition {
                    name: name.clone(),
                    variable_type: TypeNode::NonNull(Box::new(TypeNode::Named(
                        "Boolean".to_string(),
                    ))),
                    default_value: None,
                });
            name
        });

        self.operation_selection_items
            .push(SelectionItem::Field(FieldSelection {
                name: "_entities".to_string(),
                selections: representative.entities_selection.clone(),
                alias: Some(alias.clone()),
                arguments: Some(
                    (
                        "representations".to_string(),
                        Value::Variable(representations_variable_name.clone()),
                    )
                        .into(),
                ),
                skip_if: None,
                include_if: run_variable_name.clone(),
                omit_from_response: false,
            }));

        if let Some(alias_paths) = custom_scalar_paths_for_entities_selection(
            &representative.entities_selection,
            supergraph,
        ) {
            self.custom_scalar_paths
                .children
                .insert(alias.clone(), alias_paths);
        }

        self.batched_aliases.push(EntityBatchAlias {
            alias,
            representations_variable_name,
            path_groups,
            requires: RequiresSelectionSet::from(&representative.requires),
            input_rewrites: representative.input_rewrites.clone(),
            output_rewrites: representative.output_rewrites.clone(),
            context: representative.context.clone(),
            run_variable_name,
        });

        Ok(())
    }

    /// A path runs when any of the fetches that reach it does. Paths that run under the same
    /// condition form a group, in the order they first show up.
    fn collect_path_groups(shape_group: &[EntityFetch]) -> Vec<PathGroup> {
        // when each fetch that reaches a path runs. They're normalized once per path below, not
        // once per fetch.
        let mut paths: IndexMap<&FlattenNodePath, Vec<RunIf>, RandomState> =
            IndexMap::with_capacity_and_hasher(shape_group.len(), RandomState::default());
        for candidate in shape_group {
            paths
                .entry(&candidate.flatten_path)
                .or_default()
                .push(candidate.run_if.clone());
        }

        let mut groups: IndexMap<RunIf, Vec<FlattenNodePath>, RandomState> = IndexMap::default();
        for (path, run_ifs) in paths {
            groups
                .entry(RunIf::or(run_ifs))
                .or_default()
                .push(path.clone());
        }

        groups
            .into_iter()
            .map(|(run_if, paths)| PathGroup {
                run_if,
                // never across groups, the folded path would run under one condition
                paths: fold_type_conditions(paths).into(),
            })
            .collect()
    }

    fn get_or_create_representations_var(
        &mut self,
        representative: &EntityFetch,
        path_groups: &[PathGroup],
    ) -> String {
        // the order of the paths doesn't change what's collected, so it's not part of the key
        let sorted = |paths: &MergePaths| {
            let mut paths = paths.as_slice().to_vec();
            paths.sort_unstable();
            paths
        };
        let mut merge_paths: Vec<FlattenNodePath> = path_groups
            .iter()
            .flat_map(|group| group.paths.as_slice().iter().cloned())
            .collect();
        merge_paths.sort_unstable();
        let path_groups = (path_groups.len() > 1).then(|| {
            let mut groups: Vec<_> = path_groups
                .iter()
                .map(|group| (group.run_if.clone(), sorted(&group.paths)))
                .collect();
            groups.sort_by(|(left, _), (right, _)| left.cmp(right));
            groups
        });
        let representations_input_key = RepresentationsInputKey {
            requires: representative.requires.clone(),
            input_rewrites: representative.input_rewrites.clone(),
            context: representative.context.clone(),
            merge_paths,
            path_groups,
        };

        if let Some(existing_name) = self
            .representations_var_by_input_key
            .get(&representations_input_key)
        {
            return existing_name.clone();
        }

        let name = next_unique_variable_name(
            "__batch_reps_",
            &mut self.used_variable_names,
            &mut self.representations_var_index,
        );

        self.operation_variable_definitions
            .push(VariableDefinition {
                name: name.clone(),
                // [_Any!]!
                variable_type: TypeNode::NonNull(
                    // [
                    Box::new(TypeNode::List(
                        Box::new(TypeNode::NonNull(
                            // _Any
                            Box::new(TypeNode::Named("_Any".to_string())),
                        )),
                        // !
                    )), // ]
                ), // !
                default_value: None,
            });

        self.representations_var_by_input_key
            .insert(representations_input_key, name.clone());

        name
    }

    fn finish(
        mut self,
        first_candidate: &EntityFetch,
        supergraph: &SupergraphState,
        max_aliases: usize,
    ) -> Result<BatchFetchNode, QueryPlanError> {
        self.operation_variable_definitions
            .extend(self.merged_non_representation_variables.iter().cloned());

        let operation_definition = OperationDefinition {
            name: None,
            operation_kind: Some(OperationKind::Query),
            selection_set: SelectionSet {
                items: self.operation_selection_items,
            },
            variable_definitions: Some(self.operation_variable_definitions),
        };

        let document = minify_operation(operation_definition, supergraph).map_err(|error| {
            QueryPlanError::Internal(format!(
                "Failed to minify batched entities operation: {error}"
            ))
        })?;

        let mut operation = PlanningFetchOperation::from_anonymous_operation(document);
        if self
            .batched_aliases
            .iter()
            .any(|alias| !FetchContext::variables(&alias.context).is_empty())
        {
            // every group of contextual argument values repeats all batch aliases, each with its
            // own representations and context values
            let group_variables = self
                .batched_aliases
                .iter()
                .flat_map(|alias| {
                    FetchContext::group_variables(&alias.context)
                        .into_iter()
                        .chain([alias.representations_variable_name.as_str()])
                        .map(str::to_string)
                })
                .collect();
            operation.operation.set_max_context_groups(
                max_aliases / self.batched_aliases.len().max(1),
                group_variables,
                (!self.custom_scalar_paths.is_empty()).then_some(&self.custom_scalar_paths),
            );
        }

        Ok(BatchFetchNode {
            id: first_candidate.fetch_node_id,
            service_name: first_candidate.service_name.clone(),
            variable_usages: if self.variable_usages.is_empty() {
                None
            } else {
                Some(self.variable_usages)
            },
            operation_kind: Some(OperationKind::Query),
            operation,
            custom_scalar_paths: (!self.custom_scalar_paths.is_empty())
                .then_some(self.custom_scalar_paths),
            entity_batch: EntityBatch {
                aliases: self.batched_aliases,
            },
        })
    }
}

/// Paths that only differ in the types of one type condition become one path with all of
/// those types. `a.@|[A].b` and `a.@|[B].b` become `a.@|[A|B].b`.
///
/// The executor takes a type condition as "any of these types", so the folded path reaches the
/// same entities, with fewer walks over the response.
fn fold_type_conditions(mut paths: Vec<FlattenNodePath>) -> Vec<FlattenNodePath> {
    // A fold can make paths match at another place, so go until nothing folds.
    loop {
        let before = paths.len();
        let longest = paths.iter().map(FlattenNodePath::len).max().unwrap_or(0);
        for place in 0..longest {
            paths = fold_type_conditions_at(paths, place);
        }
        if paths.len() == before {
            return paths;
        }
    }
}

/// One pass of `fold_type_conditions`, for the type condition at `place`.
/// Paths fold into the first one like them, so the order stays stable.
fn fold_type_conditions_at(paths: Vec<FlattenNodePath>, place: usize) -> Vec<FlattenNodePath> {
    let names_at = |index: usize| match &paths[index].as_slice()[place] {
        PathSegment::TypeCondition(condition) => condition.names(),
        _ => unreachable!("only paths with a type condition at `place` fold"),
    };

    // Which path each one folds into, by the segments around `place`.
    let mut first_by_rest: HashMap<(&[PathSegment], &[PathSegment]), usize> = HashMap::new();
    let mut folded_into: Vec<Option<usize>> = vec![None; paths.len()];
    for (index, path) in paths.iter().enumerate() {
        let segments = path.as_slice();
        if !matches!(segments.get(place), Some(PathSegment::TypeCondition(_))) {
            continue;
        }
        let rest = (&segments[..place], &segments[place + 1..]);
        let first = *first_by_rest.entry(rest).or_insert(index);
        if first != index {
            folded_into[index] = Some(first);
        }
    }
    if folded_into.iter().all(Option::is_none) {
        return paths;
    }

    // The types of each path that others fold into, theirs included.
    let mut names_by_first: HashMap<usize, BTreeSet<&str>> = HashMap::new();
    for (index, first) in folded_into.iter().enumerate() {
        let Some(first) = *first else {
            continue;
        };
        names_by_first
            .entry(first)
            .or_insert_with(|| names_at(first).iter().map(AsRef::as_ref).collect())
            .extend(names_at(index).iter().map(AsRef::as_ref));
    }
    let mut condition_by_first: HashMap<usize, TypeCondition> = names_by_first
        .into_iter()
        .map(|(first, names)| (first, TypeCondition::from_names(names)))
        .collect();

    paths
        .into_iter()
        .enumerate()
        .filter(|(index, _)| folded_into[*index].is_none())
        .map(|(index, path)| match condition_by_first.remove(&index) {
            None => path,
            Some(condition) => {
                let mut segments = path.as_slice().to_vec();
                segments[place] = PathSegment::TypeCondition(Box::new(condition));
                segments.into()
            }
        })
        .collect()
}

/// One extractable entity fetch for batching analysis.
///
/// It represents a node like:
/// ```ignore
/// Flatten(path: "products.@") {
///   Fetch(query { _entities(representations: $representations) { ... } })
/// }
/// ```
///
/// It contains all data we need to decide if this fetch can be batched
/// with other fetches to the same subgraph.
#[derive(Clone)]
struct EntityFetch {
    /// Original index in the Parallel block (for stable ordering).
    index: usize,
    fetch_node_id: i64,
    service_name: String,
    flatten_path: FlattenNodePath,
    variable_usages: Option<BTreeSet<String>>,
    requires: SelectionSet,
    entities_selection: SelectionSet,
    input_rewrites: Option<Vec<FetchRewrite>>,
    output_rewrites: Option<Vec<FetchRewrite>>,
    context: Option<Box<FetchContext>>,
    shape_key: ShapeKey,
    non_representations_variable_definitions: Vec<VariableDefinition>,
    /// When it runs: the `Include`s and `Skip`s it sits under, and the fetch's own `run_if`.
    run_if: RunIf,
}

impl EntityFetch {
    /// Exact shape comparison after hash key match.
    ///
    /// We verify all parts are identical, not only the hash value.
    fn eq_shape(&self, right: &EntityFetch) -> bool {
        self.shape_key == right.shape_key
            && self.requires == right.requires
            && self.entities_selection == right.entities_selection
            && self.input_rewrites == right.input_rewrites
            && self.output_rewrites == right.output_rewrites
            && self.context == right.context
    }

    fn from_node(index: usize, node: &PlanNode) -> Result<Option<Self>, QueryPlanError> {
        // A Flatten(Fetch) can sit under one-sided conditions, its paths run under all of them.
        let mut conditions = Vec::new();
        let mut node = node;
        while let PlanNode::Condition(condition_node) = node {
            let Some((include, body)) = one_sided_body(condition_node) else {
                return Ok(None);
            };
            conditions.push((condition_node.condition.as_str(), include));
            node = body;
        }

        let Some((flatten_node, fetch_node)) = batchable_entity_fetch(node) else {
            return Ok(None);
        };

        // conditions that can never all hold, nothing to batch
        let Some(conditions) = ConditionGroup::from_literals(conditions) else {
            return Ok(None);
        };
        let run_if = RunIf::any_of([conditions]).and(&fetch_node.run_if);
        if run_if == RunIf::Never {
            return Ok(None);
        }

        let Some(entities_field) = fetch_node
            .operation
            .document
            .operation
            .selection_set
            .entities_field()
        else {
            return Ok(None);
        };

        let Some(representations_var) = entities_field.representations_variable_name() else {
            return Ok(None);
        };

        let Some(requires) = fetch_node.planner_requires.clone() else {
            return Ok(None);
        };

        let requires =
            requires.inline_fragment_spreads(&fetch_node.operation.document.fragments)?;
        let entities_selection = entities_field
            .selections
            .inline_fragment_spreads(&fetch_node.operation.document.fragments)?;
        let input_rewrites = fetch_node.input_rewrites.clone();
        let output_rewrites = fetch_node.output_rewrites.clone();
        let context = fetch_node.context.clone();
        let non_representations_variable_definitions = fetch_node
            .operation
            .document
            .operation
            .variable_definitions
            .clone()
            .unwrap_or_default()
            .into_iter()
            .filter(|var| var.name != representations_var)
            .collect::<Vec<_>>();

        let fragments = &fetch_node.operation.document.fragments;

        // Compute the order-independent hash for `requires`
        let mut hasher = Xxh3::new();
        let shape_context = SemanticShapeHashContext::new(fragments);
        requires.semantic_shape_hash(&mut hasher, &shape_context);
        let requires_hash = hasher.finish();

        // Compute the order-independent hash for `_entities`
        let mut hasher = Xxh3::new();
        let shape_context = SemanticShapeHashContext::new(fragments);
        entities_selection.semantic_shape_hash(&mut hasher, &shape_context);
        let entities_selection_hash = hasher.finish();

        let shape_key = ShapeKey {
            requires_hash,
            entities_selection_hash,
            input_rewrites_hash: fetch_rewrites_hash(input_rewrites.as_deref()),
            output_rewrites_hash: fetch_rewrites_hash(output_rewrites.as_deref()),
            context_rewrites_hash: fetch_rewrites_hash(FetchContext::rewrites(&context)),
        };

        Ok(Some(EntityFetch {
            index,
            fetch_node_id: fetch_node.id,
            service_name: fetch_node.service_name.clone(),
            flatten_path: flatten_node.path.clone(),
            variable_usages: fetch_node.variable_usages.clone(),
            requires,
            entities_selection,
            input_rewrites,
            output_rewrites,
            context,
            shape_key,
            non_representations_variable_definitions,
            run_if,
        }))
    }
}

/// Candidates that can share non-representations variables.
///
/// Two fetches can be batched only when variables with the same name
/// also have the same type and default value.
///
/// This struct contains:
/// - `candidates`: fetches in this compatible group
/// - `variables`: merged variable definitions for the group
#[derive(Clone)]
struct VariableCompatibleGroup {
    candidates: Vec<EntityFetch>,
    variables: Vec<VariableDefinition>,
}

pub(super) fn optimize_top_level_sequence(nodes: Vec<PlanNode>) -> Vec<PlanNode> {
    optimize_plan_sequence(nodes)
}

pub(super) fn optimize_root_node(
    node: PlanNode,
    supergraph: &SupergraphState,
    options: &QueryPlannerOptions,
) -> Result<PlanNode, QueryPlanError> {
    // Single entry point for optimizer rewrites.
    PlanOptimizer {
        supergraph,
        options,
    }
    .optimize_node(node)
}

struct PlanOptimizer<'a> {
    supergraph: &'a SupergraphState,
    options: &'a QueryPlannerOptions,
}

impl PlanOptimizer<'_> {
    fn optimize_node(&self, node: PlanNode) -> Result<PlanNode, QueryPlanError> {
        match node {
            PlanNode::Fetch(_) | PlanNode::BatchFetch(_) => Ok(node),
            PlanNode::Flatten(flatten_node) => {
                if !matches!(flatten_node.node.as_ref(), PlanNode::Fetch(_)) {
                    return Err(QueryPlanError::Internal(format!(
                        "FlattenNode is expected to wrap a FetchNode, got {:?}",
                        flatten_node.node.as_ref()
                    )));
                }

                Ok(PlanNode::Flatten(flatten_node))
            }
            PlanNode::Sequence(mut sequence_node) => {
                sequence_node.nodes = self.optimize_children(sequence_node.nodes)?;
                sequence_node.nodes = optimize_plan_sequence(sequence_node.nodes);
                Ok(PlanNode::sequence(sequence_node.nodes))
            }
            PlanNode::Parallel(parallel_node) => {
                let nodes = lift_conditional_entity_fetches(merge_parallel_conditions(
                    PlanNode::flatten_parallel(parallel_node.nodes),
                ));
                // If all children are fetching nodes, skip recursive optimize walk.
                let optimized_nodes = if nodes.iter().all(|node| node.is_fetching_node()) {
                    nodes
                } else {
                    self.optimize_children(nodes)?
                };

                let optimized_nodes = PlanNode::flatten_parallel(optimized_nodes);
                let optimized_nodes =
                    optimize_parallel_node(optimized_nodes, self.supergraph, self.options)?;

                Ok(PlanNode::parallel(optimized_nodes))
            }
            PlanNode::Condition(mut condition_node) => {
                self.optimize_optional_child(&mut condition_node.if_clause)?;
                self.optimize_optional_child(&mut condition_node.else_clause)?;
                Ok(PlanNode::Condition(condition_node))
            }
            PlanNode::Subscription(subscription_node) => {
                // self.optimize_node on a FetchNode returns itself, no need to optimize
                Ok(PlanNode::Subscription(subscription_node))
            }
            PlanNode::Defer(mut defer_node) => {
                self.optimize_optional_child(&mut defer_node.primary.node)?;

                for deferred in defer_node.deferred.iter_mut() {
                    self.optimize_optional_child(&mut deferred.node)?;
                }

                Ok(PlanNode::Defer(defer_node))
            }
        }
    }

    fn optimize_children(&self, nodes: Vec<PlanNode>) -> Result<Vec<PlanNode>, QueryPlanError> {
        nodes
            .into_iter()
            .map(|node| self.optimize_node(node))
            .collect()
    }

    fn optimize_optional_child(
        &self,
        node: &mut Option<Box<PlanNode>>,
    ) -> Result<(), QueryPlanError> {
        if let Some(current_node) = node.take() {
            *node = Some(Box::new(self.optimize_node(*current_node)?));
        }

        Ok(())
    }
}

/// Puts the siblings of a `Parallel` under the same condition together, so the fetches under it
/// can be batched like any others:
///
/// ```ignore
/// Parallel { Include(if: $a) { X }, Fetch, Include(if: $a) { Y } }
/// ```
///
/// becomes
///
/// ```ignore
/// Parallel { Include(if: $a) { Parallel { X, Y } }, Fetch }
/// ```
///
/// Only one-sided conditions (an `Include` or a `Skip`), at the place of the first one.
fn merge_parallel_conditions(nodes: Vec<PlanNode>) -> Vec<PlanNode> {
    // Where each condition is in `merged`, by its variable and whether it's an `Include`.
    let mut places: HashMap<(String, bool), usize> = HashMap::new();
    let mut merged: Vec<PlanNode> = Vec::with_capacity(nodes.len());

    for node in nodes {
        let PlanNode::Condition(condition) = node else {
            merged.push(node);
            continue;
        };
        let Some((include, _)) = one_sided_body(&condition) else {
            merged.push(PlanNode::Condition(condition));
            continue;
        };
        let key = (condition.condition.clone(), include);
        let Some(&place) = places.get(&key) else {
            places.insert(key, merged.len());
            merged.push(PlanNode::Condition(condition));
            continue;
        };

        let PlanNode::Condition(first) = &mut merged[place] else {
            unreachable!("only conditions have a place");
        };
        let (slot, body) = if include {
            (&mut first.if_clause, condition.if_clause)
        } else {
            (&mut first.else_clause, condition.else_clause)
        };
        let nodes = slot
            .take()
            .into_iter()
            .chain(body)
            .map(|node| *node)
            .collect();
        *slot = Some(Box::new(PlanNode::parallel(PlanNode::flatten_parallel(
            nodes,
        ))));
    }

    merged
}

/// Takes entity fetches out of one-sided conditions when their subgraph is also fetched by
/// another sibling, so they can share a batch. Each one keeps the conditions it was under:
///
/// ```ignore
/// Parallel { Flatten(a) { Fetch(s) }, Include(if: $c) { Parallel { Skip(if: $d) { Flatten(b) { Fetch(s) } }, X } } }
/// ```
///
/// becomes
///
/// ```ignore
/// Parallel { Flatten(a) { Fetch(s) }, Include(if: $c) { Skip(if: $d) { Flatten(b) { Fetch(s) } } }, Include(if: $c) { X } }
/// ```
///
/// and the batch gives the alias of `b` both conditions. Fetches of a subgraph that only one
/// sibling has are left alone, they get batched when that sibling is optimized.
/// Expects the conditions to be merged first.
fn lift_conditional_entity_fetches(nodes: Vec<PlanNode>) -> Vec<PlanNode> {
    // The first sibling each subgraph shows up in.
    // Once it shows up in another one, it's worth lifting.
    let mut first_sibling: HashMap<&str, usize> = HashMap::new();
    let mut liftable: HashSet<String> = HashSet::new();

    for (index, node) in nodes.iter().enumerate() {
        for_each_entity_fetch(node, &mut |service| match first_sibling.get(service) {
            None => {
                first_sibling.insert(service, index);
            }
            Some(&first) if first != index => {
                liftable.insert(service.to_string());
            }
            Some(_) => {}
        });
    }

    if liftable.is_empty() {
        return nodes;
    }

    let mut lifted = Vec::with_capacity(nodes.len());
    for node in nodes {
        if !matches!(node, PlanNode::Condition(_)) {
            lifted.push(node);
            continue;
        }
        let mut out = Vec::new();
        let kept = take_liftable_entity_fetches(node, &liftable, &mut Vec::new(), &mut out);
        lifted.extend(out);
        lifted.extend(kept);
    }

    lifted
}

/// Calls `f` with the subgraph of each entity fetch that `node` runs, looking into
/// one-sided conditions and `Parallel`s, as those don't change when a fetch runs.
fn for_each_entity_fetch<'a>(node: &'a PlanNode, f: &mut impl FnMut(&'a str)) {
    match node {
        PlanNode::Condition(condition_node) => {
            if let Some((_, body)) = one_sided_body(condition_node) {
                for_each_entity_fetch(body, f);
            }
        }
        PlanNode::Parallel(parallel_node) => {
            for child in &parallel_node.nodes {
                for_each_entity_fetch(child, f);
            }
        }
        node => {
            if let Some(service) = entity_fetch_service(node) {
                f(service);
            }
        }
    }
}

/// Moves the entity fetches of `liftable` subgraphs out of `node` into `out`, each wrapped in the
/// conditions around it (`conditions`, outermost first). Returns what's left of `node`.
fn take_liftable_entity_fetches(
    node: PlanNode,
    liftable: &HashSet<String>,
    conditions: &mut Vec<(String, bool)>,
    out: &mut Vec<PlanNode>,
) -> Option<PlanNode> {
    match node {
        PlanNode::Condition(mut condition_node) => {
            let Some((include, _)) = one_sided_body(&condition_node) else {
                return Some(PlanNode::Condition(condition_node));
            };
            let slot = if include {
                &mut condition_node.if_clause
            } else {
                &mut condition_node.else_clause
            };
            let body = *slot.take()?;
            conditions.push((condition_node.condition.clone(), include));
            let kept = take_liftable_entity_fetches(body, liftable, conditions, out);
            conditions.pop();
            *slot = Some(Box::new(kept?));
            Some(PlanNode::Condition(condition_node))
        }
        PlanNode::Parallel(parallel_node) => {
            let kept: Vec<_> = parallel_node
                .nodes
                .into_iter()
                .filter_map(|child| take_liftable_entity_fetches(child, liftable, conditions, out))
                .collect();
            (!kept.is_empty()).then(|| PlanNode::parallel(kept))
        }
        node if !conditions.is_empty()
            && entity_fetch_service(&node).is_some_and(|service| liftable.contains(service)) =>
        {
            let wrapped = conditions
                .iter()
                .rev()
                .fold(node, |node, (variable, include)| {
                    let clause = Some(Box::new(node));
                    let (if_clause, else_clause) = if *include {
                        (clause, None)
                    } else {
                        (None, clause)
                    };
                    PlanNode::Condition(ConditionNode {
                        condition: variable.clone(),
                        if_clause,
                        else_clause,
                    })
                });
            out.push(wrapped);
            None
        }
        node => Some(node),
    }
}

/// Whether it's an `Include` (`true`) or a `Skip` (`false`), and what runs then.
/// `None` when it has both sides (or neither).
fn one_sided_body(condition: &ConditionNode) -> Option<(bool, &PlanNode)> {
    match (&condition.if_clause, &condition.else_clause) {
        (Some(body), None) => Some((true, body)),
        (None, Some(body)) => Some((false, body)),
        _ => None,
    }
}

/// The subgraph of an entity call that can be batched.
fn entity_fetch_service(node: &PlanNode) -> Option<&str> {
    batchable_entity_fetch(node).map(|(_, fetch_node)| fetch_node.service_name.as_str())
}

/// The Flatten(Fetch) of an entity call that can be batched, `None` for any other node.
fn batchable_entity_fetch(node: &PlanNode) -> Option<(&FlattenNode, &FetchNode)> {
    // We only batch Flatten(Fetch) as those represent the entity calls
    let PlanNode::Flatten(flatten_node) = node else {
        return None;
    };
    let PlanNode::Fetch(fetch_node) = flatten_node.node.as_ref() else {
        return None;
    };
    let operation = &fetch_node.operation.document.operation;
    operation
        .selection_set
        .entities_field()?
        .representations_variable_name()?;
    fetch_node.planner_requires.as_ref()?;

    // fetches with a non-null context argument are never batched, on purpose.
    // a batch with contexts is sent once per distinct set of context values. an alias
    // with no entities for that set still sits in the query text, so its context
    // variable has to be sent, and there's no value for it, so it goes out as null.
    // fine for a nullable argument, but for a non-null one the subgraph rejects the whole
    // request, failing the aliases that do have entities too.
    // declaring the variable nullable doesn't work, validation rejects a nullable
    // variable in a non-null position. dropping the empty aliases per request would
    // mean re-printing the operation at execution time and keeping the new string alive
    // for as long as the response borrows it, which costs every batch fetch for a rare
    // case. so these stay separate fetches, they still run in parallel, and you-know-who
    // doesn't batch at all anyway...
    let context_variables = FetchContext::variables(&fetch_node.context);
    let has_non_null_context = operation.variable_definitions.iter().flatten().any(|var| {
        context_variables.contains(&var.name) && matches!(var.variable_type, TypeNode::NonNull(_))
    });
    (!has_non_null_context).then_some((flatten_node, fetch_node))
}

/// Merge entity fetches in a `Parallel` block into `BatchFetch` nodes
fn optimize_parallel_node(
    nodes: Vec<PlanNode>,
    supergraph: &SupergraphState,
    options: &QueryPlannerOptions,
) -> Result<Vec<PlanNode>, QueryPlanError> {
    // Find all `Flatten(Fetch)` candidates
    // and group by subgraph.
    let candidates_by_subgraph = partition_by_subgraph(&nodes)?;

    let mut batch_node_replacements: HashMap<usize, PlanNode> = HashMap::new();
    let mut removed_indices: HashSet<usize> = HashSet::new();

    for candidates in candidates_by_subgraph.into_values() {
        if candidates.len() < 2 {
            continue;
        }

        // Split groups by variable compatibility
        for variable_group in partition_by_variables_compatibility(candidates) {
            if variable_group.candidates.len() < 2 {
                continue;
            }

            // Split groups by fetch shape
            let shape_groups = partition_by_shape(variable_group.candidates);

            let Some(first_candidate) = shape_groups.first().and_then(|group| group.first()) else {
                continue;
            };

            // every shape group is one alias, so a batch carrying more aliases than the
            // subgraph allows is split into several batches. no limit, no split.
            let max_aliases = options
                .max_aliases_per_request(&first_candidate.service_name)
                .unwrap_or(usize::MAX);

            for chunk in shape_groups.chunks(max_aliases) {
                // a single fetch has nothing to batch with, keep the original node
                if let [group] = chunk {
                    if group.len() < 2 {
                        continue;
                    }
                }

                // For each merged group, we keep the first node position and remove the rest.
                // This keeps sibling order stable and deterministic.
                let first_index = chunk[0][0].index;

                let mut variables = Vec::new();
                for candidate in chunk.iter().flatten() {
                    merge_variable_definitions(
                        &mut variables,
                        &candidate.non_representations_variable_definitions,
                    );
                }

                // Build one `BatchFetch` node from each final group
                let batch_fetch_node =
                    build_batched_fetch_node(chunk, &variables, supergraph, max_aliases)?;

                batch_node_replacements.insert(
                    first_index,
                    PlanNode::BatchFetch(Box::new(batch_fetch_node)),
                );

                for group in chunk {
                    for candidate in group {
                        // Remove all but the first candidate from the group
                        if candidate.index != first_index {
                            removed_indices.insert(candidate.index);
                        }
                    }
                }
            }
        }
    }

    if batch_node_replacements.is_empty() {
        // Nothing to replace, return the original nodes
        return Ok(nodes);
    }

    let mut optimized_nodes = Vec::with_capacity(nodes.len() - removed_indices.len());

    for (index, node) in nodes.into_iter().enumerate() {
        // If we have a replacement for this index, use it
        if let Some(replacement) = batch_node_replacements.remove(&index) {
            optimized_nodes.push(replacement);
            continue;
        }

        // Skip nodes that were merged into another batch node
        if removed_indices.contains(&index) {
            continue;
        }

        // Otherwise keep the original node
        optimized_nodes.push(node);
    }

    Ok(optimized_nodes)
}

fn partition_by_subgraph(
    nodes: &[PlanNode],
) -> Result<HashMap<String, Vec<EntityFetch>>, QueryPlanError> {
    let mut candidates_by_subgraph: HashMap<String, Vec<EntityFetch>> = HashMap::new();

    for (index, node) in nodes.iter().enumerate() {
        if let Some(candidate) = EntityFetch::from_node(index, node)? {
            candidates_by_subgraph
                .entry(candidate.service_name.clone())
                .or_default()
                .push(candidate);
        }
    }

    Ok(candidates_by_subgraph)
}

fn fetch_rewrites_hash<T: Hash>(rewrites: Option<&[T]>) -> u64 {
    let mut hasher = Xxh3::new();

    if let Some(rewrites) = rewrites {
        true.hash(&mut hasher);
        rewrites.hash(&mut hasher);
    } else {
        false.hash(&mut hasher);
    }

    hasher.finish()
}

fn next_unique_variable_name(
    prefix: &str,
    used_variable_names: &mut HashSet<String>,
    next_index: &mut usize,
) -> String {
    loop {
        let name = format!("{prefix}{next_index}");
        *next_index += 1;

        if used_variable_names.insert(name.clone()) {
            return name;
        }
    }
}

/// Group candidates by identical shape.
///
/// Shape = requires + entities_selection + input_rewrites + output_rewrites.
///
/// Same shape means the GraphQL query structure is the same,
/// so candidates can share one alias in a batched fetch.
///
/// We first group by hash key, then verify full equality.
/// This handles hash collisions safely.
///
/// Group insertion order is stable, so alias naming is stable too.
fn partition_by_shape(candidates: Vec<EntityFetch>) -> Vec<Vec<EntityFetch>> {
    let mut groups: Vec<Vec<EntityFetch>> = Vec::new();
    let mut by_key: HashMap<ShapeKey, Vec<usize>> = HashMap::new();

    'candidates_loop: for candidate in candidates {
        if let Some(indices) = by_key.get(&candidate.shape_key) {
            for &group_index in indices {
                // Compare against the first element in the group,
                // since all elements in a group have the same shape.
                if groups[group_index][0].eq_shape(&candidate) {
                    groups[group_index].push(candidate);
                    continue 'candidates_loop;
                }
            }
        }

        // Create a new group, as it's the first time we see this exact shape
        let group_index = groups.len();
        groups.push(vec![candidate]);
        by_key
            .entry(groups[group_index][0].shape_key)
            .or_default()
            .push(group_index);
    }

    groups
}

/// Group candidates by variable compatibility.
///
/// Rule: if two fetches use the same variable name,
/// that variable must have the same type and default value.
///
/// Example:
/// ```ignore
///   $locale: String + $locale: String     → compatible (same type)
///   $locale: String + $locale: String!    → NOT compatible (different types)
/// ```
///
/// Grouping is deterministic and follows fetch order.
/// Each candidate joins the first compatible group.
/// If none is compatible, it starts a new group.
///
/// Each group also stores merged variable definitions (set union).
fn partition_by_variables_compatibility(
    candidates: Vec<EntityFetch>,
) -> Vec<VariableCompatibleGroup> {
    // Rule: same variable name must keep same type/default.
    // Example:
    //   valid:   $locale: String  + $locale: String
    //   invalid: $locale: String  + $locale: String!
    let mut groups: Vec<VariableCompatibleGroup> = Vec::new();

    for candidate in candidates {
        let candidate_variables = &candidate.non_representations_variable_definitions;
        let chosen_group = groups.iter().position(|group| {
            can_merge_variable_definitions(&group.variables, candidate_variables)
        });

        if let Some(group_index) = chosen_group {
            let group = &mut groups[group_index];
            merge_variable_definitions(&mut group.variables, candidate_variables);
            group.candidates.push(candidate);
            continue;
        }

        groups.push(VariableCompatibleGroup {
            variables: candidate_variables.clone(),
            candidates: vec![candidate],
        });
    }

    groups
}

fn can_merge_variable_definitions(
    merged_variables: &[VariableDefinition],
    other_variables: &[VariableDefinition],
) -> bool {
    for other_variable in other_variables {
        let existing_variable = merged_variables
            .iter()
            .find(|v| v.name == other_variable.name);

        let Some(existing_variable) = existing_variable else {
            continue;
        };

        if !existing_variable.can_merge(other_variable) {
            return false;
        }
    }

    true
}

fn merge_variable_definitions(
    merged_variables: &mut Vec<VariableDefinition>,
    other_variables: &[VariableDefinition],
) {
    for variable in other_variables {
        if merged_variables
            .iter()
            .any(|existing| existing.name == variable.name)
        {
            continue;
        }

        merged_variables.push(variable.clone());
    }
}

/// Build one `BatchFetchNode` from shape groups.
///
/// Each shape group becomes one alias in the batched operation.
/// One alias can fill multiple merge paths in the final response.
///
/// # Example
///
/// Given shape groups:
/// ```ignore
/// ShapeGroup1: [EntityFetch{path: "products.@"}, EntityFetch{path: "users.@"}]
/// ShapeGroup2: [EntityFetch{path: "reviews.@"}]
/// ```
///
/// Resulting BatchFetch:
/// ```ignore
/// BatchFetch {
///   operation: query($__batch_reps_0: [[_Any!]!]!, $__batch_reps_1: [[_Any!]!]!) {
///     _entities(representations: $__batch_reps_0) { ... }
///     _entities(representations: $__batch_reps_1) { ... }
///   }
///   entityBatch {
///     aliases: [
///       { alias: "_e0", paths: ["products.@", "users.@"], ... },    // ShapeGroup1
///       { alias: "_e1", paths: ["reviews.@"], ... },                // ShapeGroup2
///     ]
///   }
/// }
/// ```
///
/// The executor calls `_entities` once per alias.
/// Then it spreads results to all paths covered by that alias.
fn build_batched_fetch_node(
    shape_groups: &[Vec<EntityFetch>],
    merged_non_representation_variables: &[VariableDefinition],
    supergraph: &SupergraphState,
    max_aliases: usize,
) -> Result<BatchFetchNode, QueryPlanError> {
    let first_candidate = shape_groups
        .first()
        .and_then(|group| group.first())
        .ok_or_else(|| {
            QueryPlanError::Internal("Batched entities candidates were empty".to_string())
        })?;

    let mut builder =
        BatchFetchBuilder::new(merged_non_representation_variables, shape_groups.len());

    for (index, shape_group) in shape_groups.iter().enumerate() {
        builder.add_shape_group(index, shape_group, supergraph)?;
    }

    builder.finish(first_candidate, supergraph, max_aliases)
}

fn optimize_plan_sequence(nodes: Vec<PlanNode>) -> Vec<PlanNode> {
    // Flatten nested Sequence nodes.
    // If a Sequence contains another Sequence, we can inline inner nodes,
    // because execution order stays the same.
    let mut flattened_nodes = Vec::with_capacity(nodes.len());

    for node in nodes {
        match node {
            PlanNode::Sequence(sequence_node) => {
                flattened_nodes.extend(sequence_node.nodes);
            }
            other => flattened_nodes.push(other),
        }
    }

    // Merge consecutive compatible Condition nodes.
    // We check if current condition can be merged into the previous one.
    flattened_nodes
        .into_iter()
        .fold(Vec::new(), |mut acc, current_node| {
            match (acc.last_mut(), current_node) {
                // Merge adjacent compatible Condition nodes.
                (Some(PlanNode::Condition(last_cond)), PlanNode::Condition(current_cond))
                    if last_cond.can_merge_with(&current_cond) =>
                {
                    last_cond.merge(current_cond);
                }
                (_, current_node) => {
                    acc.push(current_node);
                }
            }
            acc
        })
}

#[cfg(test)]
mod tests {
    use ahash::HashSet;
    use std::{collections::BTreeSet, fs, path::PathBuf, rc::Rc};

    use graphql_tools::parser::query as query_ast;

    use crate::executor::{
        execution::demand_control::{
            compile_actual_subgraph_cost_plan,
            estimate_actual_subgraph_response_cost_with_compiled_plan,
            subgraph_response_tracker::SubgraphResponseCostTracker,
        },
        response::value::Value,
    };
    use crate::query_planner::planner::plan_nodes::RunIf;
    use crate::query_planner::{
        ast::{
            document::Document,
            merge_path::{FieldPathSegment, MergePath, Segment},
            operation::PlanningFetchOperation,
            requires::RequiresSelectionSet,
            selection_item::SelectionItem,
        },
        planner::plan_nodes::planning::{
            ConditionNode, FetchNode, FetchNodePathSegment, FetchRewrite, FlattenNode,
            FlattenNodePath, PathSegment, PlanNode, QueryPlan, TypeCondition, ValueSetter,
        },
        planner::QueryPlannerOptions,
        state::supergraph_state::{OperationKind, SupergraphState},
        utils::parsing::{parse_operation, parse_schema},
    };

    use super::{
        fold_type_conditions, lift_conditional_entity_fetches, next_unique_variable_name,
        optimize_root_node,
    };

    #[test]
    fn next_unique_representations_variable_name_skips_used_values() {
        let mut used =
            HashSet::from_iter(["__batch_reps_0".to_string(), "__batch_reps_2".to_string()]);
        let mut next_index = 0;

        let first = next_unique_variable_name("__batch_reps_", &mut used, &mut next_index);
        let second = next_unique_variable_name("__batch_reps_", &mut used, &mut next_index);

        assert_eq!(first, "__batch_reps_1");
        assert_eq!(second, "__batch_reps_3");
    }

    /// Two compatible entity fetches should become one BatchFetch
    /// and must keep node order stable
    #[test]
    fn optimize_parallel_node_batches_compatible_fetches_and_keeps_order() {
        let supergraph = test_supergraph_state();
        let requires_query = "query { ... on Product { upc } }";
        let entities_query = "
          query($representations:[_Any!]!) {
            _entities(representations: $representations) {
              ... on Product { shippingEstimate }
            }
          }
        ";

        let passthrough = PlanNode::Fetch(Box::new(non_entity_fetch_node(100, "products")));
        let candidate_a =
            flatten_entity_fetch_node(1, "inventory", "products", requires_query, &entities_query);
        let candidate_b =
            flatten_entity_fetch_node(2, "inventory", "products", requires_query, &entities_query);

        let nodes = vec![passthrough.clone(), candidate_a, candidate_b];
        let optimized =
            optimize_root_node(PlanNode::parallel(nodes), &supergraph, &Default::default())
                .expect("optimize should work");

        let query_plan = QueryPlan {
            kind: "QueryPlan",
            node: Some(optimized),
        };

        insta::assert_snapshot!(format!("{query_plan}"), @r#"
        QueryPlan {
          Parallel {
            Fetch(service: "products") {
              {
                products {
                  upc
                }
              }
            },
            BatchFetch(service: "inventory") {
              {
                _e0 {
                  paths: [
                    "products.@"
                  ]
                  {
                    ... on Product {
                      upc
                    }
                  }
                }
              }
              {
                _e0: _entities(representations: $__batch_reps_0) {
                  ... on Product {
                    shippingEstimate
                  }
                }
              }
            },
          },
        },
        "#);
    }

    /// Same variable name with incompatible types should block batching,
    /// as merging incompatible variable definitions produces invalid operations.
    ///
    /// This case is not really a possible scenario,
    /// as the variables are coming from the original query,
    /// so there won't be any conflicting variable names.
    ///
    /// It's worth asserting this anyway, to ensure the optimizer behaves correctly.
    #[test]
    fn optimize_parallel_node_does_not_batch_incompatible_variables() {
        let supergraph = test_supergraph_state();
        let requires_query = "query { ... on Product { upc } }";

        // String vs String!
        let entities_query_a = "
          query($representations:[_Any!]!, $locale: String)  {
            _entities(representations: $representations) {
              ... on Product { shippingEstimate }
            }
          }
        ";
        let entities_query_b = "
          query($representations:[_Any!]!, $locale: String!) {
            _entities(representations: $representations) {
              ... on Product { shippingEstimate }
            }
          }
        ";

        let candidate_a = flatten_entity_fetch_node(
            1,
            "inventory",
            "products",
            requires_query,
            &entities_query_a,
        );
        let candidate_b = flatten_entity_fetch_node(
            2,
            "inventory",
            "products",
            requires_query,
            &entities_query_b,
        );

        let nodes = vec![candidate_a, candidate_b];
        let optimized =
            optimize_root_node(PlanNode::parallel(nodes), &supergraph, &Default::default())
                .expect("optimize should work");

        let query_plan = QueryPlan {
            kind: "QueryPlan",
            node: Some(optimized),
        };

        insta::assert_snapshot!(format!("{query_plan}"), @r#"
        QueryPlan {
          Parallel {
            Flatten(path: "products.@") {
              Fetch(service: "inventory") {
                {
                  ... on Product {
                    upc
                  }
                } =>
                ($locale:String) {
                  ... on Product {
                    shippingEstimate
                  }
                }
              },
            },
            Flatten(path: "products.@") {
              Fetch(service: "inventory") {
                {
                  ... on Product {
                    upc
                  }
                } =>
                ($locale:String!) {
                  ... on Product {
                    shippingEstimate
                  }
                }
              },
            },
          },
        },
        "#);
    }

    /// Generated `__batch_reps_*` variable must skip already-used operation variables,
    /// to avoid collision that would make the generated query invalid.
    #[test]
    fn optimize_parallel_node_avoids_representations_variable_collision() {
        let supergraph = test_supergraph_state();
        let requires_query = "query { ... on Product { upc } }";
        let entities_query = "
          query($representations:[_Any!]!, $__batch_reps_0: String) {
            _entities(representations: $representations) {
              ... on Product { shippingEstimate }
            }
          }
        ";

        let candidate_a =
            flatten_entity_fetch_node(1, "inventory", "products", requires_query, &entities_query);
        let candidate_b =
            flatten_entity_fetch_node(2, "inventory", "products", requires_query, &entities_query);

        let nodes = vec![candidate_a, candidate_b];
        let optimized =
            optimize_root_node(PlanNode::parallel(nodes), &supergraph, &Default::default())
                .expect("optimize should work");

        let query_plan = QueryPlan {
            kind: "QueryPlan",
            node: Some(optimized),
        };

        insta::assert_snapshot!(format!("{query_plan}"), @r#"
        QueryPlan {
          BatchFetch(service: "inventory") {
            {
              _e0 {
                paths: [
                  "products.@"
                ]
                {
                  ... on Product {
                    upc
                  }
                }
              }
            }
            ($__batch_reps_0:String) {
              _e0: _entities(representations: $__batch_reps_1) {
                ... on Product {
                  shippingEstimate
                }
              }
            }
          },
        },
        "#);
    }

    /// Compatible entity fetches from different subgraphs must not be batched.
    #[test]
    fn optimize_parallel_node_does_not_batch_across_subgraphs() {
        let supergraph = test_supergraph_state();
        let requires_query = "query { ... on Product { upc } }";
        let entities_query = "
          query($representations:[_Any!]!) {
            _entities(representations: $representations) {
              ... on Product { shippingEstimate }
            }
          }
        ";

        let inventory_candidate = flatten_entity_fetch_node(
            1,
            "inventory", // different subgraph
            "products",
            requires_query,
            entities_query,
        );
        let products_candidate = flatten_entity_fetch_node(
            2,
            "products", // different subgraph
            "products",
            requires_query,
            entities_query,
        );

        let nodes = vec![inventory_candidate, products_candidate];
        let optimized =
            optimize_root_node(PlanNode::parallel(nodes), &supergraph, &Default::default())
                .expect("optimize should work");

        let query_plan = QueryPlan {
            kind: "QueryPlan",
            node: Some(optimized),
        };

        insta::assert_snapshot!(format!("{query_plan}"), @r#"
        QueryPlan {
          Parallel {
            Flatten(path: "products.@") {
              Fetch(service: "inventory") {
                {
                  ... on Product {
                    upc
                  }
                } =>
                {
                  ... on Product {
                    shippingEstimate
                  }
                }
              },
            },
            Flatten(path: "products.@") {
              Fetch(service: "products") {
                {
                  ... on Product {
                    upc
                  }
                } =>
                {
                  ... on Product {
                    shippingEstimate
                  }
                }
              },
            },
          },
        },
        "#);
    }

    /// Entity fetches with the same shape share one alias, whatever their conditions. A path
    /// runs when any of the fetches that reach it does: `products.@` is under `$a`, `!$a` and
    /// `$b`, so it always runs, and `topProducts.@` gets a group of its own under `$a`.
    #[test]
    fn optimize_parallel_node_batches_fetches_under_the_same_condition() {
        let supergraph = test_supergraph_state();
        let requires_query = "query { ... on Product { upc } }";
        let entities_query = "
          query($representations:[_Any!]!) {
            _entities(representations: $representations) {
              ... on Product { shippingEstimate }
            }
          }
        ";
        let fetch = |id, path| {
            flatten_entity_fetch_node(id, "inventory", path, requires_query, entities_query)
        };
        let condition = |variable: &str, include: bool, node: PlanNode| {
            let node = Some(Box::new(node));
            PlanNode::Condition(ConditionNode {
                condition: variable.to_string(),
                if_clause: if include { node.clone() } else { None },
                else_clause: if include { None } else { node },
            })
        };

        let nodes = vec![
            condition("a", true, fetch(1, "products")),
            condition("a", false, fetch(2, "products")),
            condition("b", true, fetch(3, "products")),
            condition("a", true, fetch(4, "topProducts")),
        ];
        let optimized =
            optimize_root_node(PlanNode::parallel(nodes), &supergraph, &Default::default())
                .expect("optimize should work");

        let query_plan = QueryPlan {
            kind: "QueryPlan",
            node: Some(optimized),
        };

        insta::assert_snapshot!(format!("{query_plan}"), @r#"
        QueryPlan {
          BatchFetch(service: "inventory") {
            {
              _e0 {
                paths: [
                  "products.@"
                ]
                paths if $a: [
                  "topProducts.@"
                ]
                {
                  ... on Product {
                    upc
                  }
                }
              }
            }
            {
              _e0: _entities(representations: $__batch_reps_0) {
                ... on Product {
                  shippingEstimate
                }
              }
            }
          },
        },
        "#);
    }

    /// A fetch with a non-null context argument is never batched, so lifting it out of its
    /// condition would only split the condition for nothing.
    #[test]
    fn lift_leaves_entity_fetches_that_cannot_be_batched_in_their_condition() {
        let requires_query = "query { ... on Product { upc } }";
        let entities_query = "
          query($representations:[_Any!]!) {
            _entities(representations: $representations) {
              ... on Product { shippingEstimate }
            }
          }
        ";
        let context_query = "
          query($representations:[_Any!]!, $__context_0: String!) {
            _entities(representations: $representations) {
              ... on Product { shippingEstimate(code: $__context_0) }
            }
          }
        ";
        let PlanNode::Flatten(mut with_context) =
            flatten_entity_fetch_node(2, "inventory", "products", requires_query, context_query)
        else {
            unreachable!("the helper makes a Flatten");
        };
        let PlanNode::Fetch(fetch_node) = with_context.node.as_mut() else {
            unreachable!("the helper makes a Fetch");
        };
        fetch_node.context = crate::query_planner::planner::plan_nodes::FetchContext::new(
            vec!["__context_0".to_string()],
            None,
        );

        let body = PlanNode::parallel(vec![
            PlanNode::Flatten(with_context),
            PlanNode::Fetch(Box::new(non_entity_fetch_node(3, "products"))),
        ]);
        let nodes = vec![
            flatten_entity_fetch_node(1, "inventory", "products", requires_query, entities_query),
            PlanNode::Condition(ConditionNode {
                condition: "c".to_string(),
                if_clause: Some(Box::new(body)),
                else_clause: None,
            }),
        ];

        let lifted = lift_conditional_entity_fetches(nodes);

        assert_eq!(lifted.len(), 2, "the condition should stay in one piece");
        let PlanNode::Condition(condition) = &lifted[1] else {
            panic!("expected the condition to stay");
        };
        assert!(matches!(
            condition.if_clause.as_deref(),
            Some(PlanNode::Parallel(parallel)) if parallel.nodes.len() == 2
        ));
    }

    /// Two aliases that read the same paths share a representations variable, whatever order
    /// their fetches came in. Each one has a single group, all its paths run together.
    #[test]
    fn optimize_parallel_node_shares_representations_of_paths_in_another_order() {
        let supergraph = test_supergraph_state();
        let requires_query = "query { ... on Product { upc } }";
        let entities_query = |field: &str| {
            format!(
                "query($representations:[_Any!]!) {{
                  _entities(representations: $representations) {{ ... on Product {{ {field} }} }}
                }}"
            )
        };
        // the `Include`s ask for one field and the `Skip`s for another, so they're two aliases
        let fetch = |id, path| {
            let field = if id % 2 == 1 {
                "shippingEstimate"
            } else {
                "inStock"
            };
            flatten_entity_fetch_node(
                id,
                "inventory",
                path,
                requires_query,
                &entities_query(field),
            )
        };
        let condition = |include: bool, node: PlanNode| {
            let node = Some(Box::new(node));
            PlanNode::Condition(ConditionNode {
                condition: "a".to_string(),
                if_clause: if include { node.clone() } else { None },
                else_clause: if include { None } else { node },
            })
        };

        // `Include` reads products first, `Skip` reads top products first
        let nodes = vec![
            condition(true, fetch(1, "products")),
            condition(false, fetch(2, "topProducts")),
            condition(true, fetch(3, "topProducts")),
            condition(false, fetch(4, "products")),
        ];
        let optimized =
            optimize_root_node(PlanNode::parallel(nodes), &supergraph, &Default::default())
                .expect("optimize should work");

        let batch_node = optimized
            .as_batch_fetch()
            .expect("optimized should be BatchFetch node");
        let aliases = &batch_node.entity_batch.aliases;
        assert_eq!(aliases.len(), 2);
        assert_ne!(aliases[0].path_groups, aliases[1].path_groups);
        assert_eq!(
            aliases[0].representations_variable_name,
            aliases[1].representations_variable_name
        );
    }

    /// The fetch graph (`batch_multi_type`) doesn't merge entity steps whose types differ at
    /// more than one place, or that have two type conditions in a row (`|[I]|[A]`). Here both
    /// happen, so 8 fetches with the same shape come in, one per type combination. Their paths
    /// fold into 2:
    ///
    /// ```text
    /// items.@|[I]|[A|B].thing|[X|Y]
    /// items.@|[I]|[A|B].other.thing|[X|Y]
    /// ```
    ///
    /// The wfg `query-1` fixture had 4,564 paths like this, 201 once folded. The executor walks
    /// each one from the root of the response, once for representations and once for results.
    #[test]
    fn optimize_parallel_node_folds_paths_that_differ_in_one_type_condition() {
        let supergraph = test_supergraph_state();
        let requires_query = "query { ... on Product { upc } }";
        let entities_query = "
          query($representations:[_Any!]!) {
            _entities(representations: $representations) {
              ... on Product { shippingEstimate }
            }
          }
        ";
        let field =
            |name: &str| Segment::Field(FieldPathSegment::named(name.to_string()).into(), 0, None);
        let types = |names: &[&str]| {
            Segment::TypeCondition(
                Rc::new(names.iter().map(|name| name.to_string()).collect()),
                None,
            )
        };
        // items.@|[I]|[<outer>].<middle..>.thing|[<inner>]
        let fetch = |id, outer: &str, middle: &[&str], inner: &str| {
            let mut segments = vec![
                field("items"),
                Segment::List,
                types(&["I"]),
                types(&[outer]),
            ];
            segments.extend(middle.iter().map(|name| field(name)));
            segments.extend([field("thing"), types(&[inner])]);

            let PlanNode::Flatten(mut flatten_node) =
                flatten_entity_fetch_node(id, "inventory", "items", requires_query, entities_query)
            else {
                unreachable!("the helper makes a Flatten");
            };
            flatten_node.path = FlattenNodePath::from(&MergePath::new(segments));
            PlanNode::Flatten(flatten_node)
        };

        let mut nodes = Vec::new();
        for middle in [&[][..], &["other"][..]] {
            for outer in ["A", "B"] {
                for inner in ["X", "Y"] {
                    nodes.push(fetch(nodes.len() as i64, outer, middle, inner));
                }
            }
        }
        let optimized =
            optimize_root_node(PlanNode::parallel(nodes), &supergraph, &Default::default())
                .expect("optimize should work");

        let query_plan = QueryPlan {
            kind: "QueryPlan",
            node: Some(optimized),
        };

        insta::assert_snapshot!(format!("{query_plan}"), @r#"
        QueryPlan {
          BatchFetch(service: "inventory") {
            {
              _e0 {
                paths: [
                  "items.@|[I]|[A|B].thing|[X|Y]"
                  "items.@|[I]|[A|B].other.thing|[X|Y]"
                ]
                {
                  ... on Product {
                    upc
                  }
                }
              }
            }
            {
              _e0: _entities(representations: $__batch_reps_0) {
                ... on Product {
                  shippingEstimate
                }
              }
            }
          },
        },
        "#);
    }

    /// Paths only fold with paths of their own group. `[A]` always runs and `[B]`, `[C]` only
    /// under `$c`. Folding `[A|B|C]` into one path would run all of them always, or none of
    /// them without `$c`.
    #[test]
    fn optimize_parallel_node_folds_paths_only_within_their_group() {
        let supergraph = test_supergraph_state();
        let requires_query = "query { ... on Product { upc } }";
        let entities_query = "
          query($representations:[_Any!]!) {
            _entities(representations: $representations) {
              ... on Product { shippingEstimate }
            }
          }
        ";
        // items.@|[<type>].thing
        let fetch = |id, type_name: &str| {
            let segments = vec![
                Segment::Field(FieldPathSegment::named("items".to_string()).into(), 0, None),
                Segment::List,
                Segment::TypeCondition(Rc::new([type_name.to_string()].into()), None),
                Segment::Field(FieldPathSegment::named("thing".to_string()).into(), 0, None),
            ];
            let PlanNode::Flatten(mut flatten_node) =
                flatten_entity_fetch_node(id, "inventory", "items", requires_query, entities_query)
            else {
                unreachable!("the helper makes a Flatten");
            };
            flatten_node.path = FlattenNodePath::from(&MergePath::new(segments));
            PlanNode::Flatten(flatten_node)
        };
        let under_c = |node: PlanNode| {
            PlanNode::Condition(ConditionNode {
                condition: "c".to_string(),
                if_clause: Some(Box::new(node)),
                else_clause: None,
            })
        };

        let nodes = vec![
            fetch(1, "A"),
            under_c(fetch(2, "B")),
            under_c(fetch(3, "C")),
        ];
        let optimized =
            optimize_root_node(PlanNode::parallel(nodes), &supergraph, &Default::default())
                .expect("optimize should work");
        let batch_node = optimized
            .as_batch_fetch()
            .expect("optimized should be BatchFetch node");

        let groups: Vec<String> = batch_node.entity_batch.aliases[0]
            .path_groups
            .iter()
            .map(|group| {
                let paths: Vec<String> = group.paths.iter().map(|p| p.to_string()).collect();
                match &group.run_if {
                    RunIf::Always => format!("always: {}", paths.join(", ")),
                    run_if => format!("{run_if}: {}", paths.join(", ")),
                }
            })
            .collect();
        assert_eq!(
            groups,
            ["always: items.@|[A].thing", "$c: items.@|[B|C].thing"]
        );
        // `[A]` always runs, so the alias does too
        assert_eq!(batch_node.entity_batch.aliases[0].run_variable_name, None);
    }

    /// Aliases share a representations variable only when, whenever both run, they collect
    /// from the same paths: one group each, or the same groups. `_e0` and `_e1` have the same
    /// paths under swapped conditions, with `$a` on and `$b` off they'd need different lists.
    /// `_e2` has the same groups as `_e0`, so it shares with it.
    #[test]
    fn optimize_parallel_node_shares_representations_only_for_the_same_groups() {
        let supergraph = test_supergraph_state();
        let requires_query = "query { ... on Product { upc } }";
        let entities_query = |field: &str| {
            format!(
                "query($representations:[_Any!]!) {{
                  _entities(representations: $representations) {{ ... on Product {{ {field} }} }}
                }}"
            )
        };
        let fetch = |id, path, field: &str, variable: &str| {
            PlanNode::Condition(ConditionNode {
                condition: variable.to_string(),
                if_clause: Some(Box::new(flatten_entity_fetch_node(
                    id,
                    "inventory",
                    path,
                    requires_query,
                    &entities_query(field),
                ))),
                else_clause: None,
            })
        };

        let nodes = vec![
            fetch(1, "products", "shippingEstimate", "a"),
            fetch(2, "topProducts", "shippingEstimate", "b"),
            fetch(3, "products", "inStock", "b"),
            fetch(4, "topProducts", "inStock", "a"),
            fetch(5, "products", "upc", "a"),
            fetch(6, "topProducts", "upc", "b"),
        ];
        let optimized =
            optimize_root_node(PlanNode::parallel(nodes), &supergraph, &Default::default())
                .expect("optimize should work");
        let aliases = &optimized
            .as_batch_fetch()
            .expect("optimized should be BatchFetch node")
            .entity_batch
            .aliases;

        assert_eq!(aliases.len(), 3);
        let variables: Vec<&str> = aliases
            .iter()
            .map(|alias| alias.representations_variable_name.as_str())
            .collect();
        assert_eq!(
            variables,
            ["__batch_reps_0", "__batch_reps_1", "__batch_reps_0"]
        );
        assert!(aliases
            .iter()
            .all(|alias| alias.run_variable_name.is_some()));
    }

    /// Paths only fold when they differ in one type condition. `[C].x|[Y]` differs from the
    /// others in two places, folding it in would reach `[C].x|[X]` and `[A].x|[Y]` too.
    #[test]
    fn fold_type_conditions_keeps_paths_that_differ_in_two_type_conditions() {
        // `@` is a list step, `|A|B` a type condition, anything else a field.
        let path = |steps: &[&str]| -> FlattenNodePath {
            steps
                .iter()
                .map(|step| match *step {
                    "@" => PathSegment::List,
                    s if s.starts_with('|') => PathSegment::TypeCondition(Box::new(
                        TypeCondition::from_names(s.trim_start_matches('|').split('|')),
                    )),
                    field => PathSegment::Field(field.into()),
                })
                .collect::<Vec<_>>()
                .into()
        };

        let folded = fold_type_conditions(vec![
            path(&["a", "@", "|A", "x", "|X"]),
            path(&["a", "@", "|B", "x", "|X"]),
            path(&["a", "@", "|C", "x", "|Y"]),
        ]);

        let folded: Vec<String> = folded.iter().map(ToString::to_string).collect();
        assert_eq!(folded, ["a.@|[A|B].x|[X]", "a.@|[C].x|[Y]"]);
    }

    /// Candidates with the same shape should batch even when merge paths differ.
    #[test]
    fn optimize_parallel_node_batches_same_shape_with_different_merge_paths() {
        let supergraph = test_supergraph_state();
        let requires_query = "query { ... on Product { upc } }";
        let entities_query = "
          query($representations:[_Any!]!) {
            _entities(representations: $representations) {
              ... on Product { shippingEstimate }
            }
          }
        ";

        let products_candidate = flatten_entity_fetch_node(
            1,
            "inventory",
            "products", // different merge paths
            requires_query,
            entities_query,
        );
        let top_products_candidate = flatten_entity_fetch_node(
            2,
            "inventory",
            "topProducts", // different merge paths
            requires_query,
            entities_query,
        );

        let nodes = vec![products_candidate, top_products_candidate];
        let optimized =
            optimize_root_node(PlanNode::parallel(nodes), &supergraph, &Default::default())
                .expect("optimize should work");

        let query_plan = QueryPlan {
            kind: "QueryPlan",
            node: Some(optimized),
        };

        insta::assert_snapshot!(format!("{query_plan}"), @r#"
        QueryPlan {
          BatchFetch(service: "inventory") {
            {
              _e0 {
                paths: [
                  "products.@"
                  "topProducts.@"
                ]
                {
                  ... on Product {
                    upc
                  }
                }
              }
            }
            {
              _e0: _entities(representations: $__batch_reps_0) {
                ... on Product {
                  shippingEstimate
                }
              }
            }
          },
        },
        "#);
    }

    /// Different entities selections should not be merged into the same alias.
    /// They should still be batched in one BatchFetch with multiple aliases.
    #[test]
    fn optimize_parallel_node_splits_aliases_when_entities_query_differs() {
        let supergraph = test_supergraph_state();
        let requires_query = "query { ... on Product { upc } }";
        let shipping_estimate_query = "
          query($representations:[_Any!]!) {
            _entities(representations: $representations) {
              ... on Product { shippingEstimate }
            }
          }
        ";
        let in_stock_query = "
          query($representations:[_Any!]!) {
            _entities(representations: $representations) {
              ... on Product { inStock }
            }
          }
        ";

        let products_candidate = flatten_entity_fetch_node(
            1,
            "inventory",
            "products",
            requires_query,
            shipping_estimate_query,
        );
        let top_products_candidate = flatten_entity_fetch_node(
            2,
            "inventory",
            "topProducts",
            requires_query,
            in_stock_query,
        );

        let nodes = vec![products_candidate, top_products_candidate];

        let optimized =
            optimize_root_node(PlanNode::parallel(nodes), &supergraph, &Default::default())
                .expect("optimize should work");

        let query_plan = QueryPlan {
            kind: "QueryPlan",
            node: Some(optimized),
        };

        insta::assert_snapshot!(format!("{query_plan}"), @r#"
        QueryPlan {
          BatchFetch(service: "inventory") {
            {
              _e0 {
                paths: [
                  "products.@"
                ]
                {
                  ... on Product {
                    upc
                  }
                }
              }
              _e1 {
                paths: [
                  "topProducts.@"
                ]
                {
                  ... on Product {
                    upc
                  }
                }
              }
            }
            {
              _e0: _entities(representations: $__batch_reps_0) {
                ... on Product {
                  shippingEstimate
                }
              }
              _e1: _entities(representations: $__batch_reps_1) {
                ... on Product {
                  inStock
                }
              }
            }
          },
        },
        "#);
    }

    /// Splitting fetches with identical operations but different requirements must preserve
    /// the total actual subgraph cost across all returned entities.
    #[test]
    fn optimize_parallel_node_preserves_actual_subgraph_cost_when_capping_aliases() {
        let supergraph = test_supergraph_state();
        let entities_query = "
          query($representations:[_Any!]!) {
            _entities(representations: $representations) {
              ... on Product { shippingEstimate }
            }
          }
        ";
        let mut totals = Vec::new();
        for max_aliases_per_request in [None, Some(1)] {
            let nodes = ["upc", "name"]
                .iter()
                .enumerate()
                .map(|(index, key)| {
                    flatten_entity_fetch_node(
                        index as i64,
                        "inventory",
                        "products",
                        &format!("query {{ ... on Product {{ {key} }} }}"),
                        entities_query,
                    )
                })
                .collect();
            let options = QueryPlannerOptions {
                max_aliases_per_request,
                ..Default::default()
            };
            let optimized = optimize_root_node(PlanNode::parallel(nodes), &supergraph, &options)
                .expect("optimize should work");
            let nodes = match optimized {
                PlanNode::Parallel(parallel) => parallel.nodes,
                node => vec![node],
            };
            assert_eq!(
                nodes.len(),
                if max_aliases_per_request == Some(1) {
                    2
                } else {
                    1
                },
                "the cap should split the batch into two separate fetches"
            );
            let mut tracker = SubgraphResponseCostTracker::new();
            let mut hashes = Vec::new();
            for node in &nodes {
                let operation = match node {
                    PlanNode::BatchFetch(batch) => &batch.operation,
                    PlanNode::Flatten(flatten) => {
                        let PlanNode::Fetch(fetch) = flatten.node.as_ref() else {
                            panic!("flatten should wrap a Fetch node");
                        };
                        &fetch.operation
                    }
                    _ => panic!("optimized nodes should be entity fetches"),
                };
                hashes.push(operation.operation.hash);
                let response = Value::Object(
                    operation
                        .document
                        .operation
                        .selection_set
                        .items
                        .iter()
                        .map(|selection| {
                            let SelectionItem::Field(field) = selection else {
                                panic!("entity operations should have top-level fields");
                            };
                            (
                                field.alias.as_deref().unwrap_or(&field.name),
                                Value::Array(vec![Value::Object(vec![(
                                    "shippingEstimate",
                                    Value::I64(1),
                                )])]),
                            )
                        })
                        .collect(),
                );
                let cost_plan = compile_actual_subgraph_cost_plan(&operation.document, &supergraph);
                let cost = estimate_actual_subgraph_response_cost_with_compiled_plan(
                    &cost_plan, &response, &None,
                );
                assert!(
                    cost > 0,
                    "each entity response should contribute to actual cost"
                );
                tracker.track("inventory", operation.operation.hash, cost);
            }
            if max_aliases_per_request == Some(1) {
                assert_eq!(
                    hashes[0], hashes[1],
                    "separate fetches should share an operation hash"
                );
            }
            assert_eq!(
                tracker.total(),
                2,
                "both returned Product entities should be counted"
            );
            totals.push(tracker.total());
        }
        assert_eq!(
            totals[0], totals[1],
            "capping aliases must preserve actual subgraph cost"
        );
    }

    #[test]
    fn optimize_parallel_node_declares_only_variables_used_in_each_chunk() {
        let supergraph = test_supergraph_state();
        let requires = "query { ... on Product { upc } }";
        let queries = [
            "query($representations:[_Any!]!) { _entities(representations:$representations) { ... on Product { shippingEstimate } } }",
            "query($representations:[_Any!]!) { _entities(representations:$representations) { ... on Product { inStock } } }",
            "query($representations:[_Any!]!, $b:Boolean!) { _entities(representations:$representations) { ... on Product { shippingEstimate @include(if:$b) } } }",
        ];
        let nodes = [queries[0], queries[1], queries[2], queries[2]]
            .iter()
            .enumerate()
            .map(|(index, query)| {
                flatten_entity_fetch_node(index as i64, "inventory", "products", requires, query)
            })
            .collect();
        let options = QueryPlannerOptions {
            max_aliases_per_request: Some(2),
            ..Default::default()
        };
        let optimized =
            optimize_root_node(PlanNode::parallel(nodes), &supergraph, &options).unwrap();
        let PlanNode::Parallel(parallel) = optimized else {
            panic!("parallel");
        };
        assert_eq!(parallel.nodes.len(), 2);
        for (index, node) in parallel.nodes.iter().enumerate() {
            let PlanNode::BatchFetch(batch) = node else {
                panic!("batch");
            };
            let definitions = batch
                .operation
                .document
                .operation
                .variable_definitions
                .as_ref()
                .unwrap();
            assert_eq!(
                definitions.iter().any(|variable| variable.name == "b"),
                index == 1
            );
        }
    }

    /// Without `max_aliases_per_request` a batch is never split, however many shapes it has.
    #[test]
    fn optimize_parallel_node_does_not_split_batches_without_max_aliases_per_request() {
        let supergraph = test_supergraph_state();
        let nodes = (0..20)
            .map(|index| {
                flatten_entity_fetch_node(
                    index,
                    "inventory",
                    "products",
                    "query { ... on Product { upc } }",
                    &format!(
                        "query($representations:[_Any!]!) {{ _entities(representations: $representations) {{ ... on Product {{ a{index}: inStock }} }} }}"
                    ),
                )
            })
            .collect();
        let optimized = optimize_root_node(
            PlanNode::parallel(nodes),
            &supergraph,
            &QueryPlannerOptions::default(),
        )
        .unwrap();
        let PlanNode::BatchFetch(batch) = optimized else {
            panic!("expected a single batch, got {optimized:?}");
        };
        assert_eq!(batch.entity_batch.aliases.len(), 20);
    }

    /// A batch with more aliases than `max_aliases_per_request` is split into several batches,
    /// and a leftover single fetch stays a plain fetch.
    #[test]
    fn optimize_parallel_node_splits_batches_over_max_aliases_per_request() {
        let supergraph = test_supergraph_state();
        let requires_upc = "query { ... on Product { upc } }";
        let requires_name = "query { ... on Product { name } }";
        let shipping_estimate_query = "
          query($representations:[_Any!]!) {
            _entities(representations: $representations) {
              ... on Product { shippingEstimate }
            }
          }
        ";
        let in_stock_query = "
          query($representations:[_Any!]!) {
            _entities(representations: $representations) {
              ... on Product { inStock }
            }
          }
        ";

        let nodes = vec![
            flatten_entity_fetch_node(
                1,
                "inventory",
                "products",
                requires_upc,
                shipping_estimate_query,
            ),
            flatten_entity_fetch_node(2, "inventory", "products", requires_upc, in_stock_query),
            flatten_entity_fetch_node(
                3,
                "inventory",
                "products",
                requires_name,
                shipping_estimate_query,
            ),
        ];

        let options = QueryPlannerOptions {
            subgraph_max_aliases_per_request: [("inventory".to_string(), 2)].into(),
            ..Default::default()
        };
        let optimized = optimize_root_node(PlanNode::parallel(nodes), &supergraph, &options)
            .expect("optimize should work");

        let query_plan = QueryPlan {
            kind: "QueryPlan",
            node: Some(optimized),
        };

        insta::assert_snapshot!(format!("{query_plan}"), @r#"
        QueryPlan {
          Parallel {
            BatchFetch(service: "inventory") {
              {
                _e0 {
                  paths: [
                    "products.@"
                  ]
                  {
                    ... on Product {
                      upc
                    }
                  }
                }
                _e1 {
                  paths: [
                    "products.@"
                  ]
                  {
                    ... on Product {
                      upc
                    }
                  }
                }
              }
              {
                _e0: _entities(representations: $__batch_reps_0) {
                  ... on Product {
                    shippingEstimate
                  }
                }
                _e1: _entities(representations: $__batch_reps_0) {
                  ... on Product {
                    inStock
                  }
                }
              }
            },
            Flatten(path: "products.@") {
              Fetch(service: "inventory") {
                {
                  ... on Product {
                    name
                  }
                } =>
                {
                  ... on Product {
                    shippingEstimate
                  }
                }
              },
            },
          },
        },
        "#);
    }

    /// Aliases with same input side and same paths can share one representations variable.
    #[test]
    fn optimize_parallel_node_shares_representations_variable_when_input_and_paths_match() {
        let supergraph = test_supergraph_state();
        let requires_query = "query { ... on Product { upc } }";
        let shipping_query = "
          query($representations:[_Any!]!) {
            _entities(representations: $representations) {
              ... on Product { shippingEstimate }
            }
          }
        ";
        let in_stock_query = "
          query($representations:[_Any!]!) {
            _entities(representations: $representations) {
              ... on Product { inStock }
            }
          }
        ";

        let a =
            flatten_entity_fetch_node(1, "inventory", "products", requires_query, shipping_query);
        let b =
            flatten_entity_fetch_node(2, "inventory", "products", requires_query, in_stock_query);

        let optimized = optimize_root_node(
            PlanNode::parallel(vec![a, b]),
            &supergraph,
            &Default::default(),
        )
        .expect("optimize should work");

        let query_plan = QueryPlan {
            kind: "QueryPlan",
            node: Some(optimized),
        };

        insta::assert_snapshot!(format!("{query_plan}"), @r#"
        QueryPlan {
          BatchFetch(service: "inventory") {
            {
              _e0 {
                paths: [
                  "products.@"
                ]
                {
                  ... on Product {
                    upc
                  }
                }
              }
              _e1 {
                paths: [
                  "products.@"
                ]
                {
                  ... on Product {
                    upc
                  }
                }
              }
            }
            {
              _e0: _entities(representations: $__batch_reps_0) {
                ... on Product {
                  shippingEstimate
                }
              }
              _e1: _entities(representations: $__batch_reps_0) {
                ... on Product {
                  inStock
                }
              }
            }
          },
        },
        "#);
    }

    /// Different requires should prevent sharing the representations variable.
    #[test]
    fn optimize_parallel_node_does_not_share_representations_variable_when_requires_differs() {
        let supergraph = test_supergraph_state();
        let requires_upc = "query { ... on Product { upc } }";
        let requires_name = "query { ... on Product { name } }";
        let entities_query = "
          query($representations:[_Any!]!) {
            _entities(representations: $representations) {
              ... on Product { shippingEstimate }
            }
          }
        ";

        let a = flatten_entity_fetch_node(1, "inventory", "products", requires_upc, entities_query);
        let b =
            flatten_entity_fetch_node(2, "inventory", "products", requires_name, entities_query);

        let optimized = optimize_root_node(
            PlanNode::parallel(vec![a, b]),
            &supergraph,
            &Default::default(),
        )
        .expect("optimize should work");

        let query_plan = QueryPlan {
            kind: "QueryPlan",
            node: Some(optimized),
        };

        insta::assert_snapshot!(format!("{query_plan}"), @r#"
        QueryPlan {
          BatchFetch(service: "inventory") {
            {
              _e0 {
                paths: [
                  "products.@"
                ]
                {
                  ... on Product {
                    upc
                  }
                }
              }
              _e1 {
                paths: [
                  "products.@"
                ]
                {
                  ... on Product {
                    name
                  }
                }
              }
            }
            {
              _e0: _entities(representations: $__batch_reps_0) {
                ... on Product {
                  ...a
                }
              }
              _e1: _entities(representations: $__batch_reps_1) {
                ... on Product {
                  ...a
                }
              }
            }
            fragment a on Product {
              shippingEstimate
            }
          },
        },
        "#);
    }

    /// Different input rewrites should prevent sharing the representations variable.
    #[test]
    fn optimize_parallel_node_does_not_share_representations_variable_when_input_rewrites_differ() {
        let supergraph = test_supergraph_state();
        let requires_query = "query { ... on Product { upc } }";
        let entities_query = "
          query($representations:[_Any!]!) {
            _entities(representations: $representations) {
              ... on Product { shippingEstimate }
            }
          }
        ";

        let base =
            flatten_entity_fetch_node(1, "inventory", "products", requires_query, entities_query);
        let with_rewrite = with_input_rewrite(flatten_entity_fetch_node(
            2,
            "inventory",
            "products",
            requires_query,
            entities_query,
        ));

        let optimized = optimize_root_node(
            PlanNode::parallel(vec![base, with_rewrite]),
            &supergraph,
            &Default::default(),
        )
        .expect("optimize should work");

        let query_plan = QueryPlan {
            kind: "QueryPlan",
            node: Some(optimized),
        };

        insta::assert_snapshot!(format!("{query_plan}"), @r#"
        QueryPlan {
          BatchFetch(service: "inventory") {
            {
              _e0 {
                paths: [
                  "products.@"
                ]
                {
                  ... on Product {
                    upc
                  }
                }
              }
              _e1 {
                paths: [
                  "products.@"
                ]
                {
                  ... on Product {
                    upc
                  }
                }
              }
            }
            {
              _e0: _entities(representations: $__batch_reps_0) {
                ... on Product {
                  ...a
                }
              }
              _e1: _entities(representations: $__batch_reps_1) {
                ... on Product {
                  ...a
                }
              }
            }
            fragment a on Product {
              shippingEstimate
            }
          },
        },
        "#);
    }

    /// Different requires selections should split aliases, even with same entities query.
    #[test]
    fn optimize_parallel_node_splits_aliases_when_requires_differs() {
        let supergraph = test_supergraph_state();
        let requires_upc = "query { ... on Product { upc } }";
        let requires_name = "query { ... on Product { name } }";
        let entities_query = "
          query($representations:[_Any!]!) {
            _entities(representations: $representations) {
              ... on Product { shippingEstimate }
            }
          }
        ";

        let a = flatten_entity_fetch_node(1, "inventory", "products", requires_upc, entities_query);
        let b =
            flatten_entity_fetch_node(2, "inventory", "topProducts", requires_name, entities_query);

        let nodes = vec![a, b];

        let optimized =
            optimize_root_node(PlanNode::parallel(nodes), &supergraph, &Default::default())
                .expect("optimize should work");

        let query_plan = QueryPlan {
            kind: "QueryPlan",
            node: Some(optimized),
        };

        insta::assert_snapshot!(format!("{query_plan}"), @r#"
        QueryPlan {
          BatchFetch(service: "inventory") {
            {
              _e0 {
                paths: [
                  "products.@"
                ]
                {
                  ... on Product {
                    upc
                  }
                }
              }
              _e1 {
                paths: [
                  "topProducts.@"
                ]
                {
                  ... on Product {
                    name
                  }
                }
              }
            }
            {
              _e0: _entities(representations: $__batch_reps_0) {
                ... on Product {
                  ...a
                }
              }
              _e1: _entities(representations: $__batch_reps_1) {
                ... on Product {
                  ...a
                }
              }
            }
            fragment a on Product {
              shippingEstimate
            }
          },
        },
        "#);
    }

    /// Different input rewrites should split aliases.
    #[test]
    fn optimize_parallel_node_splits_aliases_when_input_rewrites_differ() {
        let supergraph = test_supergraph_state();
        let requires_query = "query { ... on Product { upc } }";
        let entities_query = "
          query($representations:[_Any!]!) {
            _entities(representations: $representations) {
              ... on Product { shippingEstimate }
            }
          }
        ";

        let base =
            flatten_entity_fetch_node(1, "inventory", "products", requires_query, entities_query);
        let with_rewrite = with_input_rewrite(flatten_entity_fetch_node(
            2,
            "inventory",
            "topProducts",
            requires_query,
            entities_query,
        ));

        let nodes = vec![base, with_rewrite];

        let optimized =
            optimize_root_node(PlanNode::parallel(nodes), &supergraph, &Default::default())
                .expect("optimize should work");

        // TODO: make sure rewrites are preserved
        let batch_node = optimized
            .as_batch_fetch()
            .expect("optimized should be BatchFetch node");
        let base_node = batch_node
            .entity_batch
            .aliases
            .get(0)
            .expect("BatchFetch node should have two aliases");
        let rewrite_node = batch_node
            .entity_batch
            .aliases
            .get(1)
            .expect("BatchFetch node should have two aliases");

        assert!(
            base_node.input_rewrites.is_none(),
            "base node should not have input rewrites"
        );
        assert!(
            rewrite_node.input_rewrites.is_some(),
            "rewrite node should have input rewrites"
        );

        let query_plan = QueryPlan {
            kind: "QueryPlan",
            node: Some(optimized),
        };

        insta::assert_snapshot!(format!("{query_plan}"), @r#"
        QueryPlan {
          BatchFetch(service: "inventory") {
            {
              _e0 {
                paths: [
                  "products.@"
                ]
                {
                  ... on Product {
                    upc
                  }
                }
              }
              _e1 {
                paths: [
                  "topProducts.@"
                ]
                {
                  ... on Product {
                    upc
                  }
                }
              }
            }
            {
              _e0: _entities(representations: $__batch_reps_0) {
                ... on Product {
                  ...a
                }
              }
              _e1: _entities(representations: $__batch_reps_1) {
                ... on Product {
                  ...a
                }
              }
            }
            fragment a on Product {
              shippingEstimate
            }
          },
        },
        "#);
    }

    /// Different output rewrites should split aliases.
    #[test]
    fn optimize_parallel_node_splits_aliases_when_output_rewrites_differ() {
        let supergraph = test_supergraph_state();
        let requires_query = "query { ... on Product { upc } }";
        let entities_query = "
          query($representations:[_Any!]!) {
            _entities(representations: $representations) {
              ... on Product { shippingEstimate }
            }
          }
        ";

        let base =
            flatten_entity_fetch_node(1, "inventory", "products", requires_query, entities_query);
        let with_rewrite = with_output_rewrite(flatten_entity_fetch_node(
            2,
            "inventory",
            "topProducts",
            requires_query,
            entities_query,
        ));

        let nodes = vec![base, with_rewrite];
        let optimized =
            optimize_root_node(PlanNode::parallel(nodes), &supergraph, &Default::default())
                .expect("optimize should work");

        let query_plan = QueryPlan {
            kind: "QueryPlan",
            node: Some(optimized),
        };

        insta::assert_snapshot!(format!("{query_plan}"), @r#"
        QueryPlan {
          BatchFetch(service: "inventory") {
            {
              _e0 {
                paths: [
                  "products.@"
                ]
                {
                  ... on Product {
                    upc
                  }
                }
              }
              _e1 {
                paths: [
                  "topProducts.@"
                ]
                {
                  ... on Product {
                    upc
                  }
                }
              }
            }
            {
              _e0: _entities(representations: $__batch_reps_0) {
                ... on Product {
                  ...a
                }
              }
              _e1: _entities(representations: $__batch_reps_1) {
                ... on Product {
                  ...a
                }
              }
            }
            fragment a on Product {
              shippingEstimate
            }
          },
        },
        "#);
    }

    /// Different output rewrites split aliases, but input-equivalent aliases
    /// should still share one representations variable.
    #[test]
    fn optimize_parallel_node_shares_representations_variable_when_only_output_rewrites_differ() {
        let supergraph = test_supergraph_state();
        let requires_query = "query { ... on Product { upc } }";
        let entities_query = "
          query($representations:[_Any!]!) {
            _entities(representations: $representations) {
              ... on Product { shippingEstimate }
            }
          }
        ";

        let base =
            flatten_entity_fetch_node(1, "inventory", "products", requires_query, entities_query);
        let with_output_rewrite = with_output_rewrite(flatten_entity_fetch_node(
            2,
            "inventory",
            "products",
            requires_query,
            entities_query,
        ));

        let nodes = vec![base, with_output_rewrite];
        let optimized =
            optimize_root_node(PlanNode::parallel(nodes), &supergraph, &Default::default())
                .expect("optimize should work");

        let query_plan = QueryPlan {
            kind: "QueryPlan",
            node: Some(optimized),
        };

        insta::assert_snapshot!(format!("{query_plan}"), @r#"
        QueryPlan {
          BatchFetch(service: "inventory") {
            {
              _e0 {
                paths: [
                  "products.@"
                ]
                {
                  ... on Product {
                    upc
                  }
                }
              }
              _e1 {
                paths: [
                  "products.@"
                ]
                {
                  ... on Product {
                    upc
                  }
                }
              }
            }
            {
              _e0: _entities(representations: $__batch_reps_0) {
                ... on Product {
                  ...a
                }
              }
              _e1: _entities(representations: $__batch_reps_0) {
                ... on Product {
                  ...a
                }
              }
            }
            fragment a on Product {
              shippingEstimate
            }
          },
        },
        "#);
    }

    /// Alias assignment should be stable and follow first-seen shape order.
    #[test]
    fn optimize_parallel_node_alias_order_is_deterministic() {
        let supergraph = test_supergraph_state();
        let requires_query = "query { ... on Product { upc } }";
        let shipping_query = "
          query($representations:[_Any!]!) {
            _entities(representations: $representations) {
              ... on Product { shippingEstimate }
            }
          }
        ";
        let in_stock_query = "
          query($representations:[_Any!]!) {
            _entities(representations: $representations) {
              ... on Product { inStock }
            }
          }
        ";

        let shape_a_first =
            flatten_entity_fetch_node(1, "inventory", "products", requires_query, shipping_query);
        let shape_b = flatten_entity_fetch_node(
            2,
            "inventory",
            "topProducts",
            requires_query,
            in_stock_query,
        );
        let shape_a_second =
            flatten_entity_fetch_node(3, "inventory", "products2", requires_query, shipping_query);

        let nodes = vec![shape_a_first, shape_b, shape_a_second];

        let optimized =
            optimize_root_node(PlanNode::parallel(nodes), &supergraph, &Default::default())
                .expect("optimize should work");

        let query_plan = QueryPlan {
            kind: "QueryPlan",
            node: Some(optimized),
        };

        insta::assert_snapshot!(format!("{query_plan}"), @r#"
        QueryPlan {
          BatchFetch(service: "inventory") {
            {
              _e0 {
                paths: [
                  "products.@"
                  "products2.@"
                ]
                {
                  ... on Product {
                    upc
                  }
                }
              }
              _e1 {
                paths: [
                  "topProducts.@"
                ]
                {
                  ... on Product {
                    upc
                  }
                }
              }
            }
            {
              _e0: _entities(representations: $__batch_reps_0) {
                ... on Product {
                  shippingEstimate
                }
              }
              _e1: _entities(representations: $__batch_reps_1) {
                ... on Product {
                  inStock
                }
              }
            }
          },
        },
        "#);
    }

    /// Entities queries with fragment spreads should still batch correctly.
    #[test]
    fn optimize_parallel_node_batches_entities_query_with_fragment_spread() {
        let supergraph = test_supergraph_state();
        let requires_query = "query { ... on Product { upc } }";
        // Fragment names are different, but shape is the same.
        let entities_query_a = "
          fragment A on Product { shippingEstimate }

          query($representations:[_Any!]!) {
            _entities(representations: $representations) {
              ... on Product { ...A }
            }
          }
        ";
        let entities_query_b = "
          query($representations:[_Any!]!) {
            _entities(representations: $representations) {
              ... on Product { ...B }
            }
          }
          fragment B on Product { shippingEstimate }
        ";

        let with_fragment_a =
            flatten_entity_fetch_node(1, "inventory", "products", requires_query, entities_query_a);
        let with_fragment_b = flatten_entity_fetch_node(
            2,
            "inventory",
            "topProducts",
            requires_query,
            entities_query_b,
        );

        let nodes = vec![with_fragment_a, with_fragment_b];

        let optimized =
            optimize_root_node(PlanNode::parallel(nodes), &supergraph, &Default::default())
                .expect("optimize should work");

        let query_plan = QueryPlan {
            kind: "QueryPlan",
            node: Some(optimized),
        };

        insta::assert_snapshot!(format!("{query_plan}"), @r#"
        QueryPlan {
          BatchFetch(service: "inventory") {
            {
              _e0 {
                paths: [
                  "products.@"
                  "topProducts.@"
                ]
                {
                  ... on Product {
                    upc
                  }
                }
              }
            }
            {
              _e0: _entities(representations: $__batch_reps_0) {
                ... on Product {
                  ... on Product {
                    shippingEstimate
                  }
                }
              }
            }
          },
        },
        "#);
    }

    /// If there are no entity candidates, optimizer must keep the original nodes.
    #[test]
    fn optimize_parallel_node_returns_original_when_no_entity_candidates() {
        let supergraph = test_supergraph_state();

        let first = PlanNode::Fetch(Box::new(non_entity_fetch_node(1, "products")));
        let second = PlanNode::Fetch(Box::new(non_entity_fetch_node(2, "inventory")));

        let nodes = vec![first, second];

        let optimized =
            optimize_root_node(PlanNode::parallel(nodes), &supergraph, &Default::default())
                .expect("optimize should work");

        let query_plan = QueryPlan {
            kind: "QueryPlan",
            node: Some(optimized),
        };

        insta::assert_snapshot!(format!("{query_plan}"), @r#"
        QueryPlan {
          Parallel {
            Fetch(service: "products") {
              {
                products {
                  upc
                }
              }
            },
            Fetch(service: "inventory") {
              {
                products {
                  upc
                }
              }
            },
          },
        },
        "#);
    }

    /// One candidate in a service cannot form a batch.
    #[test]
    fn optimize_parallel_node_does_not_batch_single_candidate() {
        let supergraph = test_supergraph_state();
        let requires_query = "query { ... on Product { upc } }";
        let entities_query = "
          query($representations:[_Any!]!) {
            _entities(representations: $representations) {
              ... on Product { shippingEstimate }
            }
          }
        ";

        let candidate =
            flatten_entity_fetch_node(1, "inventory", "products", requires_query, entities_query);

        let nodes = vec![candidate];

        let optimized =
            optimize_root_node(PlanNode::parallel(nodes), &supergraph, &Default::default())
                .expect("optimize should work");

        let query_plan = QueryPlan {
            kind: "QueryPlan",
            node: Some(optimized),
        };

        insta::assert_snapshot!(format!("{query_plan}"), @r#"
        QueryPlan {
          Flatten(path: "products.@") {
            Fetch(service: "inventory") {
              {
                ... on Product {
                  upc
                }
              } =>
              {
                ... on Product {
                  shippingEstimate
                }
              }
            },
          },
        },
        "#);
    }

    /// Optimizer can create separate BatchFetch nodes for different services.
    #[test]
    fn optimize_parallel_node_creates_multiple_batch_nodes_for_multiple_services() {
        let supergraph = test_supergraph_state();
        let requires_query = "query { ... on Product { upc } }";
        let entities_query = "
          query($representations:[_Any!]!) {
            _entities(representations: $representations) {
              ... on Product { shippingEstimate }
            }
          }
        ";

        let inventory_a =
            flatten_entity_fetch_node(1, "inventory", "products", requires_query, entities_query);
        let inventory_b = flatten_entity_fetch_node(
            2,
            "inventory",
            "topProducts",
            requires_query,
            entities_query,
        );
        let products_a =
            flatten_entity_fetch_node(3, "products", "products", requires_query, entities_query);
        let products_b =
            flatten_entity_fetch_node(4, "products", "topProducts", requires_query, entities_query);

        let nodes = vec![inventory_a, inventory_b, products_a, products_b];

        let optimized =
            optimize_root_node(PlanNode::parallel(nodes), &supergraph, &Default::default())
                .expect("optimize should work");

        let query_plan = QueryPlan {
            kind: "QueryPlan",
            node: Some(optimized),
        };

        insta::assert_snapshot!(format!("{query_plan}"), @r#"
        QueryPlan {
          Parallel {
            BatchFetch(service: "inventory") {
              {
                _e0 {
                  paths: [
                    "products.@"
                    "topProducts.@"
                  ]
                  {
                    ... on Product {
                      upc
                    }
                  }
                }
              }
              {
                _e0: _entities(representations: $__batch_reps_0) {
                  ... on Product {
                    shippingEstimate
                  }
                }
              }
            },
            BatchFetch(service: "products") {
              {
                _e0 {
                  paths: [
                    "products.@"
                    "topProducts.@"
                  ]
                  {
                    ... on Product {
                      upc
                    }
                  }
                }
              }
              {
                _e0: _entities(representations: $__batch_reps_0) {
                  ... on Product {
                    shippingEstimate
                  }
                }
              }
            },
          },
        },
        "#);
    }

    fn test_supergraph_state() -> SupergraphState {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("fixture/tests/simple-requires.supergraph.graphql");
        let sdl = fs::read_to_string(path).expect("fixture should be readable");
        let schema = parse_schema(&sdl);
        SupergraphState::new(&schema)
    }

    fn flatten_entity_fetch_node(
        id: i64,
        service_name: &str,
        root_path_field: &str,
        requires_query: &str,
        entities_query: &str,
    ) -> PlanNode {
        let requires = parse_document(requires_query).operation.selection_set;
        let entities_document = parse_document(entities_query);
        let non_representation_variable_names = {
            let representations_var = entities_document
                .operation
                .selection_set
                .entities_field()
                .and_then(|field| field.representations_variable_name())
                .expect("entities query should define representations variable");

            let usages: BTreeSet<String> = entities_document
                .operation
                .variable_definitions
                .clone()
                .unwrap_or_default()
                .into_iter()
                .filter(|var| var.name != representations_var)
                .map(|var| var.name)
                .collect();

            if usages.is_empty() {
                None
            } else {
                Some(usages)
            }
        };

        let operation = PlanningFetchOperation::from_anonymous_operation(entities_document);

        let fetch_node = FetchNode {
            id,
            service_name: service_name.to_string(),
            variable_usages: non_representation_variable_names,
            operation_kind: Some(OperationKind::Query),
            operation,
            custom_scalar_paths: None,
            requires: Some(RequiresSelectionSet::from(&requires)),
            planner_requires: Some(Box::new(requires)),
            input_rewrites: None,
            output_rewrites: None,
            context: None,
            run_if: RunIf::Always,
        };

        let path = FlattenNodePath::from(&MergePath::new(vec![
            Segment::Field(
                FieldPathSegment::named(root_path_field.to_string()).into(),
                0,
                None,
            ),
            Segment::List,
        ]));

        PlanNode::Flatten(FlattenNode {
            path,
            node: Box::new(PlanNode::Fetch(Box::new(fetch_node))),
        })
    }

    fn with_input_rewrite(node: PlanNode) -> PlanNode {
        with_fetch_rewrite(node, RewriteTarget::Input)
    }

    fn with_output_rewrite(node: PlanNode) -> PlanNode {
        with_fetch_rewrite(node, RewriteTarget::Output)
    }

    enum RewriteTarget {
        Input,
        Output,
    }

    fn with_fetch_rewrite(node: PlanNode, target: RewriteTarget) -> PlanNode {
        let PlanNode::Flatten(mut flatten_node) = node else {
            panic!("expected Flatten node")
        };
        let PlanNode::Fetch(mut fetch_node) = *flatten_node.node else {
            panic!("expected Flatten(Fetch)")
        };

        let rewrite = FetchRewrite::ValueSetter(ValueSetter {
            path: vec![FetchNodePathSegment::Key("upc".to_string())],
            set_value_to: "constant".to_string(),
        });

        match target {
            RewriteTarget::Input => fetch_node.input_rewrites = Some(vec![rewrite]),
            RewriteTarget::Output => fetch_node.output_rewrites = Some(vec![rewrite]),
        }

        flatten_node.node = Box::new(PlanNode::Fetch(fetch_node));
        PlanNode::Flatten(flatten_node)
    }

    fn non_entity_fetch_node(id: i64, service_name: &str) -> FetchNode {
        let operation = PlanningFetchOperation::from_anonymous_operation(parse_document(
            "query { products { upc } }",
        ));

        FetchNode {
            id,
            service_name: service_name.to_string(),
            variable_usages: None,
            operation_kind: Some(OperationKind::Query),
            operation,
            custom_scalar_paths: None,
            requires: None,
            planner_requires: None,
            input_rewrites: None,
            output_rewrites: None,
            context: None,
            run_if: RunIf::Always,
        }
    }

    fn parse_document(query: &str) -> Document {
        let document = parse_operation(query);
        let mut operation = None;
        let mut fragments = Vec::new();

        for definition in document.definitions {
            match definition {
                query_ast::Definition::Operation(current_operation) => {
                    if operation.is_none() {
                        operation = Some(current_operation.into());
                    }
                }
                query_ast::Definition::Fragment(fragment) => {
                    fragments.push(fragment.into());
                }
            }
        }

        Document {
            operation: operation.expect("operation definition should exist"),
            fragments,
        }
    }
}
