use std::{collections::BTreeSet, fmt::Display};

use bitflags::bitflags;
use petgraph::graph::NodeIndex;

use crate::{
    ast::{
        merge_path::{Condition, MergePath},
        operation::VariableDefinition,
        safe_merge::AliasesRecords,
    },
    planner::{
        fetch::{
            selections::FetchStepSelections,
            state::{MultiTypeFetchStep, SingleTypeFetchStep},
        },
        plan_nodes::FetchRewrite,
        tree::query_tree_node::MutationFieldPosition,
    },
    state::supergraph_state::SubgraphName,
};

bitflags! {
    #[derive(Default, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct FetchStepFlags: u8 {
        /// This fetch is for resolving a @requires directive.
        const USED_FOR_REQUIRES = 1 << 0;
        /// This fetch is for resolving a type condition on an interface.
        const USED_FOR_TYPE_CONDITION = 1 << 1;
    }
}

#[derive(Debug, Clone)]
pub struct FetchStepData<State> {
    pub service_name: SubgraphName,
    pub response_path: MergePath,
    pub input: FetchStepSelections<State>,
    pub output: FetchStepSelections<State>,
    pub kind: FetchStepKind,
    pub flags: FetchStepFlags,
    pub condition: Option<Condition>,
    pub variable_usages: Option<BTreeSet<String>>,
    pub variable_definitions: Option<Vec<VariableDefinition>>,
    pub mutation_field_position: MutationFieldPosition,
    pub input_rewrites: Option<Vec<FetchRewrite>>,
    pub output_rewrites: Option<Vec<FetchRewrite>>,
    pub internal_aliases_locations: Vec<(String, AliasesRecords)>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FetchStepKind {
    Entity,
    Root,
}

impl<State> Display for FetchStepData<State> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}]: ", self.service_name)?;

        for (def_name, selections) in self.input.iter() {
            write!(f, "{}/{} ", def_name, selections)?;
        }

        write!(f, "-> ")?;

        for (def_name, selections) in self.output.iter() {
            write!(f, "{}/{} ", def_name, selections)?;
        }

        write!(f, "at $.{}", self.response_path.join("."))?;

        if self.flags.contains(FetchStepFlags::USED_FOR_REQUIRES) {
            write!(f, " [@requires]")?;
        }

        if self.flags.contains(FetchStepFlags::USED_FOR_TYPE_CONDITION) {
            write!(f, " [no_pass_through]")?;
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

impl<State> FetchStepData<State> {
    pub fn pretty_write(
        &self,
        writer: &mut std::fmt::Formatter<'_>,
        index: NodeIndex,
    ) -> Result<(), std::fmt::Error> {
        write!(writer, "[{}] {}", index.index(), self)
    }

    pub fn is_entity_call(&self) -> bool {
        self.kind == FetchStepKind::Entity
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
}

impl<State> FetchStepData<State> {
    pub fn is_fetching_multiple_types(&self) -> bool {
        self.input.is_fetching_multiple_types() || self.output.is_fetching_multiple_types()
    }
}

impl FetchStepData<SingleTypeFetchStep> {
    pub fn into_multi_type(self) -> FetchStepData<MultiTypeFetchStep> {
        FetchStepData::<MultiTypeFetchStep> {
            service_name: self.service_name,
            response_path: self.response_path,
            input: self.input.into_multi_type(),
            output: self.output.into_multi_type(),
            kind: self.kind,
            flags: self.flags,
            condition: self.condition,
            variable_usages: self.variable_usages,
            variable_definitions: self.variable_definitions,
            mutation_field_position: self.mutation_field_position,
            input_rewrites: self.input_rewrites,
            output_rewrites: self.output_rewrites,
            internal_aliases_locations: self.internal_aliases_locations,
        }
    }
}
