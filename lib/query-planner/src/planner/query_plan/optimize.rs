//! # Query Plan Optimizer
//!
//! This module rewrites the initial plan tree into a smaller/faster one.
//!
//! ## What it does
//!
//! Merges multiple entity fetches to the same subgraph into a single batched fetch.
//! This reduces the number of http requests to subgraphs.
//!
//! ## Algorithm Overview
//!
//! When we have multiple entity fetches in a Parallel block:
//! ```ignore
//! Parallel {
//!   Flatten(path: "a") { Fetch(service: "a") }
//!   Flatten(path: "b") { Fetch(service: "a") }
//!   Flatten(path: "c") { Fetch(service: "a") }
//! }
//! ```
//!
//! The optimizer merges them into:
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
//! ## Batching Pipeline (3-level partitioning)
//!
//! We can only batch fetches that are compatible. The pipeline:
//!
//! 1. By subgraph: Can only batch fetches to the same service
//! 2. By variables: Can only batch if variable types/defaults are compatible
//! 3. By shape: Can only batch if `requires + selection + input/output rewrites` are identical
//!
//! Each shape group becomes one alias in the BatchFetchNode.
//!
//! ## Example
//!
//! Input (4 fetches to same service):
//! - F1: requires: Product, selection: { name },   vars: `$locale: String`
//! - F2: requires: Product, selection: { name },   vars: `$locale: String`
//! - F3: requires: Product, selection: { price },  vars: `$locale: String`
//! - F4: requires: User,    selection: { name },   vars: `$locale: String`
//!
//! Partitioning:
//! 1. By subgraph:     [F1, F2, F3, F4] (all same service)
//! 2. By variables:    [F1, F2, F3] (compatible $locale), [F4] (different requires)
//! 3. By shape:        [F1, F2] (same Product.name), [F3] (different selection)
//!
//! Result: F1+F2 -> one BatchFetch alias, F3 stays separate, F4 stays separate

use std::{
    collections::{BTreeSet, HashMap, HashSet},
    hash::{Hash, Hasher},
};

use xxhash_rust::xxh3::Xxh3;

use crate::{
    ast::{
        hash::ASTHash,
        minification::minify_operation,
        operation::{OperationDefinition, SubgraphFetchOperation, VariableDefinition},
        selection_item::SelectionItem,
        selection_set::{FieldSelection, SelectionSet},
        value::Value,
    },
    planner::error::QueryPlanError,
    planner::plan_nodes::{
        hash_minified_query, BatchFetchNode, EntityBatch, EntityBatchAlias, FetchRewrite,
        FlattenNodePath, PlanNode,
    },
    state::supergraph_state::{OperationKind, SupergraphState, TypeNode},
};

/// Fast pre-filter key for grouping fetches by shape.
///
/// Shape = requires + entities_selection + input_rewrites + output_rewrites.
/// Same shape = same GraphQL query structure = can share one alias in a batch.
///
/// Uses hashes for quick rejection during grouping. Exact equality is checked later
/// via `same_merge_shape()` to handle hash collisions safely.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct ShapeKey {
    requires_hash: u64,
    entities_selection_hash: u64,
    input_rewrites_hash: u64,
    output_rewrites_hash: u64,
}

/// An extractable entity fetch ready for batching analysis.
///
/// Represents a node like:
/// ```ignore
/// Flatten(path: "products.@") {
///   Fetch(query { _entities(representations: $representations) { ... } })
/// }
/// ```
///
/// Contains all data needed to determine if this fetch can be batched
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
    shape_key: ShapeKey,
    non_representations_variable_definitions: Vec<VariableDefinition>,
}

impl EntityFetch {
    /// Exact equality check for two entity fetches having the same shape.
    ///
    /// Called after hash pre-filter matched - now we verify all components
    /// are truly identical (not just same hash).
    fn eq_shape(&self, right: &EntityFetch) -> bool {
        self.shape_key == right.shape_key
            && self.requires == right.requires
            && self.entities_selection == right.entities_selection
            && self.input_rewrites == right.input_rewrites
            && self.output_rewrites == right.output_rewrites
    }

    fn from_node(index: usize, node: &PlanNode) -> Result<Option<Self>, QueryPlanError> {
        // We only batch Flatten(Fetch) as those represent the entity calls
        let PlanNode::Flatten(flatten_node) = node else {
            return Ok(None);
        };

        let PlanNode::Fetch(fetch_node) = flatten_node.node.as_ref() else {
            return Ok(None);
        };

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
        let Some(requires) = fetch_node.requires.clone() else {
            return Ok(None);
        };

        let requires =
            requires.inline_fragment_spreads(&fetch_node.operation.document.fragments)?;
        let entities_selection = entities_field
            .selections
            .inline_fragment_spreads(&fetch_node.operation.document.fragments)?;
        let input_rewrites = fetch_node.input_rewrites.clone();
        let output_rewrites = fetch_node.output_rewrites.clone();
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
        let mut hasher = Xxh3::new();
        let mut visiting = Vec::new();
        requires.semantic_shape_hash(&mut hasher, fragments, &mut visiting);
        let requires_hash = hasher.finish();

        let mut hasher = Xxh3::new();
        let mut visiting = Vec::new();
        entities_selection.semantic_shape_hash(&mut hasher, fragments, &mut visiting);
        let entities_selection_hash = hasher.finish();

        let shape_key = ShapeKey {
            requires_hash,
            entities_selection_hash,
            input_rewrites_hash: fetch_rewrites_hash(input_rewrites.as_deref()),
            output_rewrites_hash: fetch_rewrites_hash(output_rewrites.as_deref()),
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
            shape_key,
            non_representations_variable_definitions,
        }))
    }
}

/// Candidates that share compatible variable definitions.
///
/// Two fetches can be batched only if their non-representations variables
/// have the same names, types, and defaults.
///
/// This struct holds:
/// - `candidates`: the fetches that can share variables
/// - `variables`: the merged variable definitions (union of all candidate vars)
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
) -> Result<PlanNode, QueryPlanError> {
    // Single entrypoint for all optimizer rewrites.
    PlanOptimizer { supergraph }.optimize_node(node)
}

struct PlanOptimizer<'a> {
    supergraph: &'a SupergraphState,
}

impl PlanOptimizer<'_> {
    fn optimize_node(&self, node: PlanNode) -> Result<PlanNode, QueryPlanError> {
        match node {
            PlanNode::Fetch(_) | PlanNode::BatchFetch(_) => Ok(node),
            PlanNode::Flatten(mut flatten_node) => {
                // Flatten should wrap Fetch.
                // Keep a defensive fallback in case the shape changes in future.
                assert!(
                    matches!(flatten_node.node.as_ref(), PlanNode::Fetch(_)),
                    "FlattenNode is expected to wrap a FetchNode, got {:?}",
                    flatten_node.node.as_ref()
                );

                if !matches!(flatten_node.node.as_ref(), PlanNode::Fetch(_)) {
                    flatten_node.node = Box::new(self.optimize_node(*flatten_node.node)?);
                }

                Ok(PlanNode::Flatten(flatten_node))
            }
            PlanNode::Sequence(mut sequence_node) => {
                sequence_node.nodes = self.optimize_children(sequence_node.nodes)?;
                sequence_node.nodes = optimize_plan_sequence(sequence_node.nodes);
                Ok(PlanNode::sequence(sequence_node.nodes))
            }
            PlanNode::Parallel(parallel_node) => {
                // Fast path: if all children are leaf-like nodes, skip recursive walk.
                let optimized_nodes = if parallel_node
                    .nodes
                    .iter()
                    .all(|node| node.is_fetching_node())
                {
                    parallel_node.nodes
                } else {
                    self.optimize_children(parallel_node.nodes)?
                };

                let optimized_nodes = PlanNode::flatten_parallel(optimized_nodes);
                let optimized_nodes = optimize_parallel_node(optimized_nodes, self.supergraph)?;

                Ok(PlanNode::parallel(optimized_nodes))
            }
            PlanNode::Condition(mut condition_node) => {
                self.optimize_optional_child(&mut condition_node.if_clause)?;
                self.optimize_optional_child(&mut condition_node.else_clause)?;
                Ok(PlanNode::Condition(condition_node))
            }
            PlanNode::Subscription(mut subscription_node) => {
                subscription_node.primary =
                    Box::new(self.optimize_node(*subscription_node.primary)?);
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

/// Merges entity fetches in a Parallel block into BatchFetch nodes.
///
/// This is the core batching algorithm. Pipeline:
///
/// 1. Find all `Flatten(Fetch)` nodes
/// 2. Batch fetches to the same service
/// 3. Split if variable types/defaults differ
/// 4. Split if query structure differs
/// 5. Build BatchFetch: Each shape group -> one alias
///
/// We replace the first node in each group and remove the rest,
/// preserving the original sibling order for deterministic output.
fn optimize_parallel_node(
    nodes: Vec<PlanNode>,
    supergraph: &SupergraphState,
) -> Result<Vec<PlanNode>, QueryPlanError> {
    let candidates_by_subgraph = partition_by_subgraph(&nodes)?;

    let mut batch_node_replacements: HashMap<usize, PlanNode> = HashMap::new();
    let mut removed_indices: HashSet<usize> = HashSet::new();

    for candidates in candidates_by_subgraph.into_values() {
        if candidates.len() < 2 {
            continue;
        }

        for variable_group in partition_by_variables_compatibility(candidates) {
            if variable_group.candidates.len() < 2 {
                continue;
            }

            let shape_groups = partition_by_shape(variable_group.candidates);

            let Some(first_group) = shape_groups.first() else {
                continue;
            };
            let Some(first_candidate) = first_group.first() else {
                continue;
            };

            let first_index = first_candidate.index;
            let batch_fetch_node =
                build_batched_fetch_node(&shape_groups, &variable_group.variables, supergraph)?;

            batch_node_replacements.insert(first_index, PlanNode::BatchFetch(batch_fetch_node));

            for group in &shape_groups {
                for candidate in group {
                    // Remove all but the first candidate from the group
                    if candidate.index != first_index {
                        removed_indices.insert(candidate.index);
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
        // If a replacement exists for this index, use it instead of the original node
        if let Some(replacement) = batch_node_replacements.remove(&index) {
            optimized_nodes.push(replacement);
            continue;
        }

        // Skip removed indices
        if removed_indices.contains(&index) {
            continue;
        }

        // Otherwise, keep the original node
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

fn fetch_rewrites_hash(rewrites: Option<&[FetchRewrite]>) -> u64 {
    let mut hasher = Xxh3::new();

    if let Some(rewrites) = rewrites {
        true.hash(&mut hasher);
        rewrites.hash(&mut hasher);
    } else {
        false.hash(&mut hasher);
    }

    hasher.finish()
}

/// Partitions candidates into groups with identical shapes.
///
/// Shape = requires + entities_selection + input_rewrites + output_rewrites.
///
/// Same shape means the GraphQL query structure is identical, so they can
/// share one alias in a batched fetch.
///
/// Uses hash-based pre-filtering for performance, with exact equality check
/// to handle hash collisions. Maintains insertion order for stable alias naming.
fn partition_by_shape(candidates: Vec<EntityFetch>) -> Vec<Vec<EntityFetch>> {
    let mut groups: Vec<Vec<EntityFetch>> = Vec::new();
    let mut group_indices_by_shape: HashMap<ShapeKey, Vec<usize>> = HashMap::new();

    for candidate in candidates {
        let mut matched_group_index = None;

        if let Some(candidate_group_indices) = group_indices_by_shape.get(&candidate.shape_key) {
            for group_index in candidate_group_indices {
                let source = &groups[*group_index][0];
                if source.eq_shape(&candidate) {
                    matched_group_index = Some(*group_index);
                    break;
                }
            }
        }

        if let Some(group_index) = matched_group_index {
            groups[group_index].push(candidate);
            continue;
        }

        let new_group_index = groups.len();
        group_indices_by_shape
            .entry(candidate.shape_key)
            .or_default()
            .push(new_group_index);
        groups.push(vec![candidate]);
    }

    groups
}

/// Partitions candidates into groups where all candidates use compatible variable definitions.
///
/// Correctness rule: if two fetches use the same variable name,
/// that variable must have identical type and default value in both fetches.
///
/// Example:
/// ```ignore
///   $locale: String + $locale: String     → compatible (same type)
///   $locale: String + $locale: String!    → NOT compatible (different types)
/// ```
///
/// Grouping is deterministic and follows fetch-node order:
/// each candidate is assigned to the first compatible existing group
/// (in group creation order), otherwise it starts a new group.
///
/// Each group returns the merged variable definitions (union of all candidate vars).
fn partition_by_variables_compatibility(
    candidates: Vec<EntityFetch>,
) -> Vec<VariableCompatibleGroup> {
    // Correctness rule: if two fetches use the same variable name,
    // that variable must have identical type/default in both fetches.
    // Example:
    //   safe:   $locale: String + $locale: String
    //   unsafe: $locale: String + $locale: String!
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

/// Builds a BatchFetchNode from shape groups.
///
/// Each shape group becomes one alias in the batched operation. The alias
/// can "patch" (fill in) multiple merge paths in the final response.
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
/// The executor will call _entities once for each alias, then distribute
/// the results to all paths that alias covers.
fn build_batched_fetch_node(
    shape_groups: &[Vec<EntityFetch>],
    merged_non_representation_variables: &[VariableDefinition],
    supergraph: &SupergraphState,
) -> Result<BatchFetchNode, QueryPlanError> {
    // One alias per shape group. Each alias can patch multiple merge paths.
    // Example:
    //   _e0 -> paths ["topProducts.@", "users.@"]
    // means one _entities call feeds two places in the final response.
    let original_fetch_count = shape_groups.iter().map(|group| group.len()).sum::<usize>();
    let mut operation_variable_definitions =
        Vec::with_capacity(shape_groups.len() + merged_non_representation_variables.len());
    let mut operation_selection_items = Vec::with_capacity(shape_groups.len());
    let mut batched_aliases = Vec::with_capacity(shape_groups.len());
    let mut used_variable_names: HashSet<String> = merged_non_representation_variables
        .iter()
        .map(|var| var.name.clone())
        .collect();
    let mut representations_var_index: usize = 0;
    let mut variable_usages = BTreeSet::new();

    for (index, shape_group) in shape_groups.iter().enumerate() {
        let representative = shape_group.first().ok_or_else(|| {
            QueryPlanError::Internal("Batched entities shape group cannot be empty".to_string())
        })?;

        for candidate in shape_group {
            if let Some(candidate_variable_usages) = &candidate.variable_usages {
                variable_usages.extend(candidate_variable_usages.iter().cloned());
            }
        }

        let alias = format!("_e{index}");
        let mut representations_variable_name = format!("__batch_reps_{representations_var_index}");
        representations_var_index += 1;

        while used_variable_names.contains(&representations_variable_name) {
            representations_variable_name = format!("__batch_reps_{representations_var_index}");
            representations_var_index += 1;
        }

        used_variable_names.insert(representations_variable_name.clone());

        operation_variable_definitions.push(VariableDefinition {
            name: representations_variable_name.clone(),
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

        operation_selection_items.push(SelectionItem::Field(FieldSelection {
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
            include_if: None,
        }));

        let merge_paths = shape_group
            .iter()
            .map(|candidate| candidate.flatten_path.clone())
            .collect();

        batched_aliases.push(EntityBatchAlias {
            alias,
            representations_variable_name,
            merge_paths,
            requires: representative.requires.clone(),
            entities_selection: representative.entities_selection.clone(),
            input_rewrites: representative.input_rewrites.clone(),
            output_rewrites: representative.output_rewrites.clone(),
        });
    }

    operation_variable_definitions.extend(merged_non_representation_variables.iter().cloned());

    let operation_definition = OperationDefinition {
        name: None,
        operation_kind: Some(OperationKind::Query),
        selection_set: SelectionSet {
            items: operation_selection_items,
        },
        variable_definitions: Some(operation_variable_definitions),
    };

    let document = minify_operation(operation_definition, supergraph).map_err(|error| {
        QueryPlanError::Internal(format!(
            "Failed to minify batched entities operation: {error}"
        ))
    })?;

    let document_str = document.to_string();
    let hash = hash_minified_query(&document_str);

    let first_candidate = shape_groups
        .first()
        .and_then(|group| group.first())
        .ok_or_else(|| {
            QueryPlanError::Internal("Batched entities candidates were empty".to_string())
        })?;

    Ok(BatchFetchNode {
        id: first_candidate.fetch_node_id,
        service_name: first_candidate.service_name.clone(),
        variable_usages: if variable_usages.is_empty() {
            None
        } else {
            Some(variable_usages)
        },
        operation_kind: Some(OperationKind::Query),
        operation_name: None,
        operation: SubgraphFetchOperation {
            document,
            document_str,
            hash,
        },
        entity_batch: EntityBatch {
            original_fetch_count,
            aliases: batched_aliases,
        },
    })
}

fn optimize_plan_sequence(nodes: Vec<PlanNode>) -> Vec<PlanNode> {
    // Flatten nested Sequence groups.
    // If a Sequence contains another Sequence, we can pull the inner sequence's nodes
    // into the outer one since they are executed in the same order.
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
    // We iterate through the flattened nodes and check if the current node can be
    // merged into the previous one.
    flattened_nodes
        .into_iter()
        .fold(Vec::new(), |mut acc, current_node| {
            match (acc.last_mut(), current_node) {
                // If both the last processed node and the current node are Conditions,
                // and they target the same requirements/variables, we merge them.
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
