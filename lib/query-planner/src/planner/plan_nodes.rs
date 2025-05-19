use super::fetch::fetch_graph::FetchStepData;
use crate::{
    ast::{merge_path::MergePath, selection_set::SelectionSet},
    state::supergraph_state::RootOperationType,
    utils::pretty_display::{get_indent, PrettyDisplay},
};
use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter as FmtFormatter, Result as FmtResult};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryPlan {
    pub root: QueryPlanNode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum QueryPlanNode {
    Sequence(Vec<QueryPlanNode>),
    Parallel(Vec<QueryPlanNode>),
    Fetch(FetchNode),
    Flatten(FlattenNode),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetchNode {
    pub service_name: String,
    pub operation: String,
    pub operation_type: RootOperationType,
    pub requires: Option<SelectionSet>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FlattenNodePathType {
    Named(String),
    Indexed(usize),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlattenNode {
    pub path: Vec<FlattenNodePathType>,
    pub node: Box<QueryPlanNode>,
}

impl QueryPlanNode {
    pub fn into_nodes(self) -> Vec<QueryPlanNode> {
        match self {
            QueryPlanNode::Sequence(nodes) | QueryPlanNode::Parallel(nodes) => nodes,
            other => vec![other],
        }
    }
}

impl From<MergePath> for Vec<FlattenNodePathType> {
    fn from(path: MergePath) -> Self {
        path.inner
            .iter()
            .map(|segment| FlattenNodePathType::Named(segment.clone()))
            .collect()
    }
}

impl From<&FetchStepData> for FetchNode {
    fn from(step: &FetchStepData) -> Self {
        FetchNode {
            service_name: step.service_name.0.clone(),
            operation: step.output.selection_set.to_string(),
            // TODO: make sure it's correct
            operation_type: RootOperationType::Query,
            requires: match step.input.selection_set.is_empty() {
                true => None,
                false => Some(step.input.selection_set.clone()),
            },
        }
    }
}

impl From<&FetchStepData> for QueryPlanNode {
    fn from(step: &FetchStepData) -> Self {
        if step.response_path.is_empty() {
            QueryPlanNode::Fetch(step.into())
        } else {
            QueryPlanNode::Flatten(FlattenNode {
                // it's cheaper to clone response_path (Arc etc), rather then cloning the step
                path: step.response_path.clone().into(),
                node: Box::new(QueryPlanNode::Fetch(step.into())),
            })
        }
    }
}

impl Display for QueryPlan {
    fn fmt(&self, f: &mut FmtFormatter<'_>) -> FmtResult {
        self.pretty_fmt(f, 0)
    }
}

impl Display for QueryPlanNode {
    fn fmt(&self, f: &mut FmtFormatter<'_>) -> FmtResult {
        self.pretty_fmt(f, 0)
    }
}

impl Display for FetchNode {
    fn fmt(&self, f: &mut FmtFormatter<'_>) -> FmtResult {
        self.pretty_fmt(f, 0)
    }
}

impl Display for FlattenNode {
    fn fmt(&self, f: &mut FmtFormatter<'_>) -> FmtResult {
        self.pretty_fmt(f, 0)
    }
}

impl PrettyDisplay for QueryPlan {
    fn pretty_fmt(&self, f: &mut FmtFormatter<'_>, depth: usize) -> FmtResult {
        let indent = get_indent(depth);
        writeln!(f, "{indent}QueryPlan {{",)?;
        self.root.pretty_fmt(f, depth + 1)?;
        writeln!(f, "{indent}}},")?;
        Ok(())
    }
}

impl PrettyDisplay for FetchNode {
    fn pretty_fmt(&self, f: &mut FmtFormatter<'_>, depth: usize) -> FmtResult {
        let indent = get_indent(depth);
        writeln!(f, "{indent}Fetch(service: \"{}\") {{", self.service_name)?;
        if let Some(requires) = &self.requires {
            requires.pretty_fmt(f, depth + 2)?;
            writeln!(f, "{indent}  }} =>")?;
        }
        writeln!(f, "{indent}  {{")?;
        for line in self.operation.lines() {
            writeln!(f, "{indent}    {line}")?;
        }
        writeln!(f, "{indent}  }}")?;
        writeln!(f, "{indent}}},")?;

        Ok(())
    }
}

impl PrettyDisplay for FlattenNode {
    fn pretty_fmt(&self, f: &mut FmtFormatter<'_>, depth: usize) -> FmtResult {
        let indent = get_indent(depth);
        writeln!(
            f,
            "{indent}Flatten(path: \"{}\") {{",
            self.path
                .iter()
                .map(|segment| match segment {
                    FlattenNodePathType::Indexed(val) => val.to_string(),
                    FlattenNodePathType::Named(val) => val.clone(),
                })
                .collect::<Vec<String>>()
                .join(".")
        )?;
        self.node.pretty_fmt(f, depth + 1)?;
        writeln!(f, "{indent}}},")?;

        Ok(())
    }
}

impl PrettyDisplay for QueryPlanNode {
    fn pretty_fmt(&self, f: &mut FmtFormatter<'_>, depth: usize) -> FmtResult {
        match self {
            QueryPlanNode::Fetch(node) => node.pretty_fmt(f, depth),
            QueryPlanNode::Flatten(node) => node.pretty_fmt(f, depth),
            QueryPlanNode::Parallel(nodes) | QueryPlanNode::Sequence(nodes) => {
                let indent = get_indent(depth);
                let variant = if matches!(self, QueryPlanNode::Parallel(_)) {
                    "Parallel"
                } else {
                    "Sequence"
                };
                writeln!(f, "{indent}{variant} {{")?;
                for node in nodes {
                    node.pretty_fmt(f, depth + 1)?;
                }
                writeln!(f, "{indent}}},")?;
                Ok(())
            }
        }
    }
}
