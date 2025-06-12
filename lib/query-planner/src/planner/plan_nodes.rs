use super::fetch::fetch_graph::FetchStepData;
use crate::{
    ast::{
        operation::{OperationDefinition, SubgraphFetchOperation, VariableDefinition},
        selection_item::SelectionItem,
        selection_set::{FieldSelection, InlineFragmentSelection, SelectionSet},
        type_aware_selection::TypeAwareSelection,
        value::Value,
    },
    state::supergraph_state::{OperationKind, TypeNode},
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

#[allow(clippy::large_enum_variant)]
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variable_usages: Option<BTreeSet<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_kind: Option<OperationKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_name: Option<String>,
    pub operation: SubgraphFetchOperation,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requires: Option<SelectionSet>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_rewrites: Option<Vec<FetchRewrite>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_rewrites: Option<Vec<FetchRewrite>>,
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
#[serde(rename_all = "camelCase")]
pub struct KeyRenamer {
    pub path: Vec<String>,
    pub rename_key_to: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum FetchRewrite {
    ValueSetter(ValueSetter),
    KeyRenamer(KeyRenamer),
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
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

fn create_output_operation(step: &FetchStepData) -> SubgraphFetchOperation {
    let type_aware_selection = &step.output;
    let mut variables = vec![VariableDefinition {
        name: "representations".to_string(),
        variable_type: TypeNode::NonNull(Box::new(TypeNode::List(Box::new(TypeNode::NonNull(
            Box::new(TypeNode::Named("_Any".to_string())),
        ))))),
        default_value: None,
    }];

    if let Some(additional_vars) = &step.variable_definitions {
        variables.extend(additional_vars.clone());
    }

    let operation_def = OperationDefinition {
        name: None,
        operation_kind: Some(OperationKind::Query),
        variable_definitions: Some(variables),
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
                skip_if: None,
                include_if: None,
            })],
        },
    };
    let operation_str = operation_def.to_string();
    SubgraphFetchOperation {
        operation_def,
        operation_str,
    }
}

impl From<&FetchStepData> for OperationKind {
    fn from(step: &FetchStepData) -> Self {
        let type_name = step.output.type_name.as_str();

        if type_name == "Query" {
            OperationKind::Query
        } else if type_name == "Mutation" {
            OperationKind::Mutation
        } else if type_name == "Subscription" {
            OperationKind::Subscription
        } else {
            OperationKind::Query
        }
    }
}

impl From<&FetchStepData> for FetchNode {
    fn from(step: &FetchStepData) -> Self {
        match !step.is_entity_call() {
            true => {
                let operation_def = OperationDefinition {
                    name: None,
                    operation_kind: Some(step.into()),
                    selection_set: step.output.selection_set.clone(),
                    variable_definitions: step.variable_definitions.clone(),
                };

                let operation_str = operation_def.to_string();
                FetchNode {
                    service_name: step.service_name.0.clone(),
                    variable_usages: step.variable_usages.clone(),
                    operation_kind: Some(step.into()),
                    operation_name: None,
                    operation: SubgraphFetchOperation {
                        operation_def,
                        operation_str,
                    },
                    requires: None,
                    input_rewrites: None,
                    output_rewrites: None,
                }
            }
            false => FetchNode {
                service_name: step.service_name.0.clone(),
                variable_usages: step.variable_usages.clone(),
                operation_kind: Some(OperationKind::Query),
                operation_name: None,
                operation: create_output_operation(step),
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
