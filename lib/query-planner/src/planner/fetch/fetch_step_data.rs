use std::{collections::BTreeSet, fmt::Display};

use petgraph::{graph::NodeIndex, visit::EdgeRef};
use tracing::trace;

use crate::{
    ast::{
        merge_path::{Condition, MergePath},
        operation::VariableDefinition,
        safe_merge::AliasesRecords,
        type_aware_selection::{find_arguments_conflicts, TypeAwareSelection},
    },
    planner::{
        fetch::fetch_graph::FetchGraph, plan_nodes::FetchRewrite,
        tree::query_tree_node::MutationFieldPosition,
    },
    state::supergraph_state::SubgraphName,
};

#[derive(Debug, Clone)]
pub struct FetchStepData {
    pub service_name: SubgraphName,
    pub response_path: MergePath,
    pub input: TypeAwareSelection,
    pub output: TypeAwareSelection,
    pub kind: FetchStepKind,
    pub used_for_requires: bool,
    pub condition: Option<Condition>,
    pub variable_usages: Option<BTreeSet<String>>,
    pub variable_definitions: Option<Vec<VariableDefinition>>,
    pub mutation_field_position: MutationFieldPosition,
    pub input_rewrites: Option<Vec<FetchRewrite>>,
    pub output_rewrites: Option<Vec<FetchRewrite>>,
    pub internal_aliases_locations: AliasesRecords,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FetchStepKind {
    Entity,
    Root,
}

impl Display for FetchStepData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}/{} {} → {} at $.{}",
            self.input.type_name,
            self.service_name,
            self.input,
            self.output,
            self.response_path.join("."),
        )?;

        if self.used_for_requires {
            write!(f, " [@requires]")?;
        }

        if let Some(condition) = &self.condition {
            match condition {
                Condition::Include(var_name) => write!(f, " [@include(if: ${})]", var_name)?,
                Condition::Skip(var_name) => write!(f, " [@skip(if: ${})]", var_name)?,
            }
        }

        Ok(())
    }
}

impl FetchStepData {
    pub fn pretty_write(
        &self,
        writer: &mut std::fmt::Formatter<'_>,
        index: NodeIndex,
    ) -> Result<(), std::fmt::Error> {
        write!(writer, "[{}] {}", index.index(), self)
    }

    pub fn is_entity_call(&self) -> bool {
        self.input.type_name != "Query"
            && self.input.type_name != "Mutation"
            && self.input.type_name != "Subscription"
    }

    pub fn add_input_rewrite(&mut self, rewrite: FetchRewrite) {
        let rewrites = self.input_rewrites.get_or_insert_default();

        if !rewrites.contains(&rewrite) {
            rewrites.push(rewrite);
        }
    }

    pub fn add_output_rewrite(&mut self, rewrite: FetchRewrite) {
        let rewrites = self.output_rewrites.get_or_insert_default();

        if !rewrites.contains(&rewrite) {
            rewrites.push(rewrite);
        }
    }

    pub fn can_merge(
        &self,
        self_index: NodeIndex,
        other_index: NodeIndex,
        other: &Self,
        fetch_graph: &FetchGraph,
    ) -> bool {
        if self_index == other_index {
            return false;
        }

        if self.service_name != other.service_name {
            return false;
        }

        // We allow to merge root with entity calls by adding an inline fragment with the @include/@skip
        if self.is_entity_call() && other.is_entity_call() && self.condition != other.condition {
            return false;
        }

        // If both are entities, their response_paths should match,
        // as we can't merge entity calls resolving different entities
        if matches!(self.kind, FetchStepKind::Entity) && self.kind == other.kind {
            if !self.response_path.eq(&other.response_path) {
                return false;
            }
        } else {
            // otherwise we can merge
            if !other.response_path.starts_with(&self.response_path) {
                return false;
            }
        }

        let input_conflicts = find_arguments_conflicts(&self.input, &other.input);

        if !input_conflicts.is_empty() {
            trace!(
                "preventing merge of [{}]+[{}] due to input conflicts",
                self_index.index(),
                other_index.index()
            );

            return false;
        }

        // if the `other` FetchStep has a single parent and it's `this` FetchStep
        if fetch_graph.parents_of(other_index).count() == 1
            && fetch_graph
                .parents_of(other_index)
                .all(|edge| edge.source() == self_index)
        {
            return true;
        }

        // if they do not share parents, they can't be merged
        if !fetch_graph.parents_of(self_index).all(|self_edge| {
            fetch_graph
                .parents_of(other_index)
                .any(|other_edge| other_edge.source() == self_edge.source())
        }) {
            return false;
        }

        true
    }
}
