use super::fetch::fetch_graph::FetchStepData;
use crate::{
    ast::{
        operation::{OperationDefinition, SubgraphFetchOperation, TypeNode, VariableDefinition},
        selection_item::SelectionItem,
        selection_set::{FieldSelection, InlineFragmentSelection, SelectionSet},
        type_aware_selection::TypeAwareSelection,
        value::Value,
    },
    state::supergraph_state::OperationKind,
    utils::pretty_display::{get_indent, PrettyDisplay},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fmt::{Display, Formatter as FmtFormatter, Result as FmtResult},
};

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct QueryPlan {
    pub kind: String, // "QueryPlan"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node: Option<PlanNode>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "kind")]
pub enum PlanNode {
    Fetch(FetchNode),
    Sequence(SequenceNode),
    Parallel(ParallelNode),
    Flatten(FlattenNode),
    Condition(ConditionNode),
    Subscription(SubscriptionNode),
    Defer(DeferNode),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FetchNode {
    pub service_name: String,
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    pub variable_usages: BTreeSet<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_kind: Option<OperationKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_name: Option<String>,
    pub operation: SubgraphFetchOperation,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requires: Option<SelectionSet>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_rewrites: Option<Vec<InputRewrite>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_rewrites: Option<Vec<OutputRewrite>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlattenNode {
    pub path: Vec<String>,
    pub node: Box<PlanNode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SequenceNode {
    pub nodes: Vec<PlanNode>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParallelNode {
    pub nodes: Vec<PlanNode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConditionNode {
    pub condition: String, // The variable name acting as the condition
    pub if_clause: Option<Box<PlanNode>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub else_clause: Option<Box<PlanNode>>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum OutputRewrite {
    KeyRenamer(KeyRenamer),
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct KeyRenamer {
    pub path: Vec<String>,
    pub rename_key_to: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum InputRewrite {
    ValueSetter(ValueSetter),
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ValueSetter {
    pub path: Vec<String>,
    // Use serde_json::Value for the 'any' type
    pub set_value_to: serde_json::Value,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SubscriptionNode {
    pub primary: Box<PlanNode>, // Use Box to prevent size issues
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DeferNode {
    pub primary: DeferPrimary,
    pub deferred: Vec<DeferredNode>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DeferPrimary {
    pub subselection: Option<String>,
    pub node: Option<Box<PlanNode>>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DeferredNode {
    pub depends: Vec<DeferDependency>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub query_path: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subselection: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node: Option<Box<PlanNode>>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DeferDependency {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub defer_label: Option<String>,
}

impl PlanNode {
    pub fn into_nodes(self) -> Vec<PlanNode> {
        match self {
            PlanNode::Sequence(node) => node.nodes,
            PlanNode::Parallel(node) => node.nodes,
            other => vec![other],
        }
    }
}

fn create_input_selection_set(input_selections: &TypeAwareSelection) -> SelectionSet {
    SelectionSet {
        items: vec![SelectionItem::InlineFragment(InlineFragmentSelection {
            selections: input_selections.selection_set.strip_for_plan_input(),
            type_condition: input_selections.type_name.clone(),
        })],
    }
}

fn create_output_operation(type_aware_selection: &TypeAwareSelection) -> SubgraphFetchOperation {
    SubgraphFetchOperation(OperationDefinition {
        name: None,
        operation_kind: Some(OperationKind::Query),
        variable_definitions: Some(vec![VariableDefinition {
            name: "representations".to_string(),
            variable_type: TypeNode::NonNull(Box::new(TypeNode::List(Box::new(
                TypeNode::NonNull(Box::new(TypeNode::Named("_Any".to_string()))),
            )))),
        }]),
        selection_set: SelectionSet {
            items: vec![SelectionItem::Field(FieldSelection {
                name: "_entities".to_string(),
                selections: SelectionSet {
                    items: vec![SelectionItem::InlineFragment(InlineFragmentSelection {
                        selections: type_aware_selection.selection_set.clone(),
                        type_condition: type_aware_selection.type_name.clone(),
                    })],
                },
                alias: None,
                arguments: Some(
                    (
                        "representations".to_string(),
                        Value::Variable("representations".to_string()),
                    )
                        .into(),
                ),
            })],
        },
    })
}

impl From<&FetchStepData> for FetchNode {
    fn from(step: &FetchStepData) -> Self {
        match step.input.selection_set.is_empty() {
            true => FetchNode {
                service_name: step.service_name.0.clone(),
                variable_usages: step.output.selection_set.variable_usages(),
                operation_kind: Some(OperationKind::Query),
                operation_name: None,
                operation: SubgraphFetchOperation(OperationDefinition {
                    name: None,
                    operation_kind: None,
                    selection_set: step.output.selection_set.clone(),
                    variable_definitions: None,
                }),
                requires: None,
                input_rewrites: None,
                output_rewrites: None,
            },
            false => FetchNode {
                service_name: step.service_name.0.clone(),
                variable_usages: step.output.selection_set.variable_usages(),
                operation_kind: Some(OperationKind::Query),
                operation_name: None,
                operation: create_output_operation(&step.output),
                // TODO: make sure it's correct
                requires: Some(create_input_selection_set(&step.input)),
                input_rewrites: None,
                output_rewrites: None,
            },
        }
    }
}

impl From<&FetchStepData> for PlanNode {
    fn from(step: &FetchStepData) -> Self {
        if step.response_path.is_empty() {
            PlanNode::Fetch(step.into())
        } else {
            PlanNode::Flatten(FlattenNode {
                // it's cheaper to clone response_path (Arc etc), rather then cloning the step
                path: step.response_path.clone().into(),
                node: Box::new(PlanNode::Fetch(step.into())),
            })
        }
    }
}

impl Display for QueryPlan {
    fn fmt(&self, f: &mut FmtFormatter<'_>) -> FmtResult {
        self.pretty_fmt(f, 0)
    }
}

impl Display for PlanNode {
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
        if let Some(node) = &self.node {
            node.pretty_fmt(f, depth + 1)?;
        } else {
            writeln!(f, "{indent}  None,")?;
        }
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
        self.operation.pretty_fmt(f, depth)?;
        writeln!(f, "{indent}}},")?;

        Ok(())
    }
}

impl PrettyDisplay for FlattenNode {
    fn pretty_fmt(&self, f: &mut FmtFormatter<'_>, depth: usize) -> FmtResult {
        let indent = get_indent(depth);
        writeln!(f, "{indent}Flatten(path: \"{}\") {{", self.path.join("."))?;
        self.node.pretty_fmt(f, depth + 1)?;
        writeln!(f, "{indent}}},")?;

        Ok(())
    }
}

impl PrettyDisplay for SequenceNode {
    fn pretty_fmt(&self, f: &mut FmtFormatter<'_>, depth: usize) -> FmtResult {
        let indent = get_indent(depth);
        writeln!(f, "{indent}Sequence {{")?;
        for node in &self.nodes {
            node.pretty_fmt(f, depth + 1)?;
        }
        writeln!(f, "{indent}}},")?;
        Ok(())
    }
}

impl PrettyDisplay for ParallelNode {
    fn pretty_fmt(&self, f: &mut FmtFormatter<'_>, depth: usize) -> FmtResult {
        let indent = get_indent(depth);
        writeln!(f, "{indent}Parallel {{")?;
        for node in &self.nodes {
            node.pretty_fmt(f, depth + 1)?;
        }
        writeln!(f, "{indent}}},")?;
        Ok(())
    }
}

impl PrettyDisplay for PlanNode {
    fn pretty_fmt(&self, f: &mut FmtFormatter<'_>, depth: usize) -> FmtResult {
        match self {
            PlanNode::Fetch(node) => node.pretty_fmt(f, depth),
            PlanNode::Flatten(node) => node.pretty_fmt(f, depth),
            PlanNode::Sequence(node) => node.pretty_fmt(f, depth),
            PlanNode::Parallel(node) => node.pretty_fmt(f, depth),
            _ => Ok(()),
        }
    }
}
