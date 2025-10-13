use crate::{
    ast::{
        merge_path::{Condition, MergePath, Segment},
        minification::minify_operation,
        operation::{OperationDefinition, SubgraphFetchOperation, VariableDefinition},
        selection_item::SelectionItem,
        selection_set::{FieldSelection, InlineFragmentSelection, SelectionSet},
        type_aware_selection::TypeAwareSelection,
        value::Value,
    },
    planner::fetch::fetch_step_data::FetchStepData,
    state::supergraph_state::{OperationKind, SupergraphState, TypeNode},
    utils::pretty_display::{get_indent, PrettyDisplay},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fmt::{Display, Formatter as FmtFormatter, Result as FmtResult},
};

use napi_derive::napi;

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
#[napi(object)]
pub struct QueryPlan {
    pub kind: String, // "QueryPlan"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node: Option<PlanNode>,
}

impl QueryPlan {
    pub fn fetch_nodes(&self) -> Vec<&FetchNode> {
        match self.node.as_ref() {
            Some(node) => {
                let mut list = vec![];
                Self::fetch_nodes_from_node(node, &mut list);
                list
            }
            None => vec![],
        }
    }

    fn fetch_nodes_from_node<'a>(node: &'a PlanNode, list: &mut Vec<&'a FetchNode>) {
        match node {
            PlanNode::Condition(node) => {
                if let Some(node) = node.else_clause.as_ref() {
                    Self::fetch_nodes_from_node(node.as_ref(), list);
                }
                if let Some(node) = node.if_clause.as_ref() {
                    Self::fetch_nodes_from_node(node.as_ref(), list);
                }
            }
            PlanNode::Fetch(node) => {
                list.push(node);
            }
            PlanNode::Sequence(node) => {
                for child in &node.nodes {
                    Self::fetch_nodes_from_node(child, list);
                }
            }
            PlanNode::Parallel(node) => {
                for child in &node.nodes {
                    Self::fetch_nodes_from_node(child, list);
                }
            }
            PlanNode::Flatten(node) => {
                Self::fetch_nodes_from_node(&node.node, list);
            }
            PlanNode::Subscription(node) => {
                Self::fetch_nodes_from_node(node.primary.as_ref(), list);
            }
            PlanNode::Defer(_) => {
                unreachable!("DeferNode is not supported yet");
            }
        }
    }
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
    #[serde(skip_serializing)]
    pub id: i64,
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
    pub path: FlattenNodePath,
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

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct KeyRenamer {
    pub path: Vec<FetchNodePathSegment>,
    pub rename_key_to: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum FetchNodePathSegment {
    Key(String),
    TypenameEquals(String),
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum FetchRewrite {
    ValueSetter(ValueSetter),
    KeyRenamer(KeyRenamer),
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum FlattenNodePathSegment {
    Field(String),
    Cast(String),
    #[serde(rename = "@")]
    List,
}

impl From<&MergePath> for Vec<FetchNodePathSegment> {
    fn from(value: &MergePath) -> Self {
        value
            .inner
            .iter()
            .filter_map(|path_segment| match path_segment {
                Segment::Cast(type_name, _) => {
                    Some(FetchNodePathSegment::TypenameEquals(type_name.clone()))
                }
                Segment::Field(field_name, _args_hash, _) => {
                    Some(FetchNodePathSegment::Key(field_name.clone()))
                }
                Segment::List => None,
            })
            .collect()
    }
}

impl FlattenNodePathSegment {
    pub fn to_field(&self) -> Option<&String> {
        match self {
            FlattenNodePathSegment::Field(field_name) => Some(field_name),
            _ => None,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct FlattenNodePath(Vec<FlattenNodePathSegment>);

impl FlattenNodePath {
    pub fn as_slice(&self) -> &[FlattenNodePathSegment] {
        &self.0
    }
}

impl Display for FlattenNodePathSegment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FlattenNodePathSegment::Field(field_name) => write!(f, "{}", field_name),
            FlattenNodePathSegment::Cast(type_name) => write!(f, "|[{}]", type_name),
            FlattenNodePathSegment::List => write!(f, "@"),
        }
    }
}

impl Display for FlattenNodePath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut segments_iter = self.0.iter().peekable();

        while let Some(segment) = segments_iter.next() {
            write!(f, "{}", segment)?;
            if let Some(peeked) = segments_iter.peek() {
                match peeked {
                    FlattenNodePathSegment::Cast(_) => {
                        // Don't add a dot before Cast
                    }
                    _ => write!(f, ".")?,
                }
            }
        }
        Ok(())
    }
}

impl From<&MergePath> for FlattenNodePath {
    fn from(path: &MergePath) -> Self {
        FlattenNodePath(
            path.inner
                .iter()
                .map(|seg| match seg {
                    Segment::Cast(type_name, _) => FlattenNodePathSegment::Cast(type_name.clone()),
                    Segment::Field(field_name, _args_hash, _) => {
                        FlattenNodePathSegment::Field(field_name.clone())
                    }
                    Segment::List => FlattenNodePathSegment::List,
                })
                .collect(),
        )
    }
}

impl From<MergePath> for FlattenNodePath {
    fn from(path: MergePath) -> Self {
        (&path).into()
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ValueSetter {
    pub path: Vec<FetchNodePathSegment>,
    pub set_value_to: String,
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
            skip_if: None,
            include_if: None,
        })],
    }
}

fn create_output_operation(
    step: &FetchStepData,
    supergraph: &SupergraphState,
) -> SubgraphFetchOperation {
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
                        skip_if: None,
                        include_if: None,
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

    let document = minify_operation(operation_def, supergraph).expect("Failed to minify");

    let document_str = document.to_string();
    SubgraphFetchOperation {
        document,
        document_str,
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

impl FetchNode {
    pub fn from_fetch_step(step: &FetchStepData, supergraph: &SupergraphState) -> Self {
        match step.is_entity_call() {
            true => FetchNode {
                id: step.id,
                service_name: step.service_name.0.clone(),
                variable_usages: step.variable_usages.clone(),
                operation_kind: Some(OperationKind::Query),
                operation_name: None,
                operation: create_output_operation(step, supergraph),
                requires: Some(create_input_selection_set(&step.input)),
                input_rewrites: step.input_rewrites.clone(),
                output_rewrites: step.output_rewrites.clone(),
            },
            false => {
                let operation_def = OperationDefinition {
                    name: None,
                    operation_kind: Some(step.into()),
                    selection_set: step.output.selection_set.clone(),
                    variable_definitions: step.variable_definitions.clone(),
                };
                let document =
                    minify_operation(operation_def, supergraph).expect("Failed to minify");
                let document_str = document.to_string();

                FetchNode {
                    id: step.id,
                    service_name: step.service_name.0.clone(),
                    variable_usages: step.variable_usages.clone(),
                    operation_kind: Some(step.into()),
                    operation_name: None,
                    operation: SubgraphFetchOperation {
                        document,
                        document_str,
                    },
                    requires: None,
                    input_rewrites: step.input_rewrites.clone(),
                    output_rewrites: step.output_rewrites.clone(),
                }
            }
        }
    }
}

impl PlanNode {
    pub fn from_fetch_step(step: &FetchStepData, supergraph: &SupergraphState) -> Self {
        let node = if step.response_path.is_empty() {
            PlanNode::Fetch(FetchNode::from_fetch_step(step, supergraph))
        } else {
            PlanNode::Flatten(FlattenNode {
                path: step.response_path.clone().into(),
                node: Box::new(PlanNode::Fetch(FetchNode::from_fetch_step(
                    step, supergraph,
                ))),
            })
        };

        match step.condition.as_ref() {
            Some(condition) => match condition {
                Condition::Include(var_name) => PlanNode::Condition(ConditionNode {
                    condition: var_name.clone(),
                    if_clause: Some(Box::new(node)),
                    else_clause: None,
                }),
                Condition::Skip(var_name) => PlanNode::Condition(ConditionNode {
                    condition: var_name.clone(),
                    if_clause: None,
                    else_clause: Some(Box::new(node)),
                }),
            },
            None => node,
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
            writeln!(f, "{indent}  {{")?;
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

        writeln!(f, "{indent}Flatten(path: \"{}\") {{", self.path)?;
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

impl PrettyDisplay for ConditionNode {
    fn pretty_fmt(&self, f: &mut FmtFormatter<'_>, depth: usize) -> FmtResult {
        let indent = get_indent(depth);

        match (self.if_clause.as_ref(), self.else_clause.as_ref()) {
            (Some(if_clause), None) => {
                writeln!(f, "{indent}Include(if: ${}) {{", self.condition)?;
                if_clause.pretty_fmt(f, depth + 1)?;
                writeln!(f, "{indent}}},")?;
            }
            (None, Some(else_clause)) => {
                writeln!(f, "{indent}Skip(if: ${}) {{", self.condition)?;
                else_clause.pretty_fmt(f, depth + 1)?;
                writeln!(f, "{indent}}},")?;
            }
            (Some(_if_clause), Some(_else_clause)) => {
                todo!("Implement pretty_fmt for ConditionNode with both if and else clauses");
            }
            _ => panic!("Invalid condition node"),
        }
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
            PlanNode::Condition(node) => node.pretty_fmt(f, depth),
            _ => Ok(()),
        }
    }
}
