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

use std::{
    collections::{BTreeSet, HashMap, HashSet},
    hash::{Hash, Hasher},
};

use xxhash_rust::xxh3::Xxh3;

use crate::{
    ast::{
        hash::{ASTHash, SemanticShapeHashContext},
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
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct RepresentationsInputKey {
    requires: SelectionSet,
    input_rewrites: Option<Vec<FetchRewrite>>,
    merge_paths: Vec<FlattenNodePath>,
}

struct BatchFetchBuilder<'a> {
    merged_non_representation_variables: &'a [VariableDefinition],
    operation_variable_definitions: Vec<VariableDefinition>,
    operation_selection_items: Vec<SelectionItem>,
    batched_aliases: Vec<EntityBatchAlias>,
    used_variable_names: HashSet<String>,
    representations_var_index: usize,
    variable_usages: BTreeSet<String>,
    representations_var_by_input_key: HashMap<RepresentationsInputKey, String>,
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
            variable_usages: BTreeSet::new(),
            representations_var_by_input_key: HashMap::new(),
        }
    }

    fn add_shape_group(
        &mut self,
        alias_index: usize,
        shape_group: &[EntityFetch],
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
        let merge_paths = Self::collect_merge_paths(shape_group);
        let representations_variable_name =
            self.get_or_create_representations_var(representative, &merge_paths);

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
                include_if: None,
            }));

        self.batched_aliases.push(EntityBatchAlias {
            alias,
            representations_variable_name,
            merge_paths,
            requires: representative.requires.clone(),
            input_rewrites: representative.input_rewrites.clone(),
            output_rewrites: representative.output_rewrites.clone(),
        });

        Ok(())
    }

    fn collect_merge_paths(shape_group: &[EntityFetch]) -> Vec<FlattenNodePath> {
        let mut merge_paths = Vec::with_capacity(shape_group.len());
        let mut seen_merge_paths = HashSet::with_capacity(shape_group.len());

        for candidate in shape_group {
            let path = candidate.flatten_path.clone();
            if seen_merge_paths.insert(path.clone()) {
                merge_paths.push(path);
            }
        }

        merge_paths
    }

    fn get_or_create_representations_var(
        &mut self,
        representative: &EntityFetch,
        merge_paths: &[FlattenNodePath],
    ) -> String {
        let representations_input_key = RepresentationsInputKey {
            requires: representative.requires.clone(),
            input_rewrites: representative.input_rewrites.clone(),
            merge_paths: merge_paths.to_vec(),
        };

        if let Some(existing_name) = self
            .representations_var_by_input_key
            .get(&representations_input_key)
        {
            return existing_name.clone();
        }

        let name = next_unique_representations_var_name(
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

        let document_str = document.to_string();
        let hash = hash_minified_query(&document_str);

        Ok(BatchFetchNode {
            id: first_candidate.fetch_node_id,
            service_name: first_candidate.service_name.clone(),
            variable_usages: if self.variable_usages.is_empty() {
                None
            } else {
                Some(self.variable_usages)
            },
            operation_kind: Some(OperationKind::Query),
            operation_name: None,
            operation: SubgraphFetchOperation {
                document,
                document_str,
                hash,
            },
            entity_batch: EntityBatch {
                aliases: self.batched_aliases,
            },
        })
    }
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
    shape_key: ShapeKey,
    non_representations_variable_definitions: Vec<VariableDefinition>,
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
) -> Result<PlanNode, QueryPlanError> {
    // Single entry point for optimizer rewrites.
    PlanOptimizer { supergraph }.optimize_node(node)
}

struct PlanOptimizer<'a> {
    supergraph: &'a SupergraphState,
}

impl PlanOptimizer<'_> {
    fn optimize_node(&self, node: PlanNode) -> Result<PlanNode, QueryPlanError> {
        match node {
            PlanNode::Fetch(_) | PlanNode::BatchFetch(_) => Ok(node),
            PlanNode::Flatten(flatten_node) => {
                assert!(
                    matches!(flatten_node.node.as_ref(), PlanNode::Fetch(_)),
                    "FlattenNode is expected to wrap a FetchNode, got {:?}",
                    flatten_node.node.as_ref()
                );

                Ok(PlanNode::Flatten(flatten_node))
            }
            PlanNode::Sequence(mut sequence_node) => {
                sequence_node.nodes = self.optimize_children(sequence_node.nodes)?;
                sequence_node.nodes = optimize_plan_sequence(sequence_node.nodes);
                Ok(PlanNode::sequence(sequence_node.nodes))
            }
            PlanNode::Parallel(parallel_node) => {
                // If all children are fetching nodes, skip recursive optimize walk.
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

/// Merge entity fetches in a `Parallel` block into `BatchFetch` nodes
fn optimize_parallel_node(
    nodes: Vec<PlanNode>,
    supergraph: &SupergraphState,
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

            let Some(first_group) = shape_groups.first() else {
                continue;
            };
            let Some(first_candidate) = first_group.first() else {
                continue;
            };

            // For each merged group, we keep the first node position and remove the rest.
            // This keeps sibling order stable and deterministic.
            let first_index = first_candidate.index;

            // Build one `BatchFetch` node from each final group
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

fn next_unique_representations_var_name(
    used_variable_names: &mut HashSet<String>,
    next_index: &mut usize,
) -> String {
    loop {
        let name = format!("__batch_reps_{next_index}");
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
        builder.add_shape_group(index, shape_group)?;
    }

    builder.finish(first_candidate, supergraph)
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
    use std::{
        collections::{BTreeSet, HashSet},
        fs,
        path::PathBuf,
    };

    use graphql_tools::parser::query as query_ast;

    use crate::{
        ast::{
            document::Document,
            merge_path::{MergePath, Segment},
            operation::SubgraphFetchOperation,
        },
        planner::plan_nodes::{
            hash_minified_query, FetchNode, FetchNodePathSegment, FetchRewrite, FlattenNode,
            FlattenNodePath, PlanNode, QueryPlan, ValueSetter,
        },
        state::supergraph_state::{OperationKind, SupergraphState},
        utils::parsing::{parse_operation, parse_schema},
    };

    use super::{next_unique_representations_var_name, optimize_root_node};

    #[test]
    fn next_unique_representations_variable_name_skips_used_values() {
        let mut used = HashSet::from(["__batch_reps_0".to_string(), "__batch_reps_2".to_string()]);
        let mut next_index = 0;

        let first = next_unique_representations_var_name(&mut used, &mut next_index);
        let second = next_unique_representations_var_name(&mut used, &mut next_index);

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

        let passthrough = PlanNode::Fetch(non_entity_fetch_node(100, "products"));
        let candidate_a =
            flatten_entity_fetch_node(1, "inventory", "products", requires_query, &entities_query);
        let candidate_b =
            flatten_entity_fetch_node(2, "inventory", "products", requires_query, &entities_query);

        let nodes = vec![passthrough.clone(), candidate_a, candidate_b];
        let optimized = optimize_root_node(PlanNode::parallel(nodes), &supergraph)
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
        let optimized = optimize_root_node(PlanNode::parallel(nodes), &supergraph)
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
        let optimized = optimize_root_node(PlanNode::parallel(nodes), &supergraph)
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
        let optimized = optimize_root_node(PlanNode::parallel(nodes), &supergraph)
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
        let optimized = optimize_root_node(PlanNode::parallel(nodes), &supergraph)
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

        let optimized = optimize_root_node(PlanNode::parallel(nodes), &supergraph)
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

        let optimized = optimize_root_node(PlanNode::parallel(vec![a, b]), &supergraph)
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

        let optimized = optimize_root_node(PlanNode::parallel(vec![a, b]), &supergraph)
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

        let optimized =
            optimize_root_node(PlanNode::parallel(vec![base, with_rewrite]), &supergraph)
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

        let optimized = optimize_root_node(PlanNode::parallel(nodes), &supergraph)
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

        let optimized = optimize_root_node(PlanNode::parallel(nodes), &supergraph)
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
        let optimized = optimize_root_node(PlanNode::parallel(nodes), &supergraph)
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
        let optimized = optimize_root_node(PlanNode::parallel(nodes), &supergraph)
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

        let optimized = optimize_root_node(PlanNode::parallel(nodes), &supergraph)
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

        let optimized = optimize_root_node(PlanNode::parallel(nodes), &supergraph)
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

        let first = PlanNode::Fetch(non_entity_fetch_node(1, "products"));
        let second = PlanNode::Fetch(non_entity_fetch_node(2, "inventory"));

        let nodes = vec![first, second];

        let optimized = optimize_root_node(PlanNode::parallel(nodes), &supergraph)
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

        let optimized = optimize_root_node(PlanNode::parallel(nodes), &supergraph)
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

        let optimized = optimize_root_node(PlanNode::parallel(nodes), &supergraph)
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

        let operation = build_subgraph_fetch_operation(entities_document);

        let fetch_node = FetchNode {
            id,
            service_name: service_name.to_string(),
            variable_usages: non_representation_variable_names,
            operation_kind: Some(OperationKind::Query),
            operation_name: None,
            operation,
            requires: Some(requires),
            input_rewrites: None,
            output_rewrites: None,
        };

        let path = FlattenNodePath::from(&MergePath::new(vec![
            Segment::Field(root_path_field.to_string(), 0, None),
            Segment::List,
        ]));

        PlanNode::Flatten(FlattenNode {
            path,
            node: Box::new(PlanNode::Fetch(fetch_node)),
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
        let operation =
            build_subgraph_fetch_operation(parse_document("query { products { upc } }"));

        FetchNode {
            id,
            service_name: service_name.to_string(),
            variable_usages: None,
            operation_kind: Some(OperationKind::Query),
            operation_name: None,
            operation,
            requires: None,
            input_rewrites: None,
            output_rewrites: None,
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

    fn build_subgraph_fetch_operation(document: Document) -> SubgraphFetchOperation {
        let document_str = document.to_string();

        SubgraphFetchOperation {
            hash: hash_minified_query(&document_str),
            document,
            document_str,
        }
    }
}
