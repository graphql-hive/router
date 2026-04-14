use crate::{
    ast::{
        merge_path::{Condition, MergePath, Segment},
        minification::minify_operation,
        operation::{OperationDefinition, SubgraphFetchOperation, VariableDefinition},
        selection_item::SelectionItem,
        selection_set::{FieldSelection, SelectionSet},
        value::Value,
    },
    planner::fetch::{
        fetch_step_data::FetchStepData, selections::FetchStepSelections, state::MultiTypeFetchStep,
    },
    state::supergraph_state::{OperationKind, SupergraphState, TypeNode},
    utils::pretty_display::{get_indent, PrettyDisplay},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fmt::{Display, Formatter as FmtFormatter, Result as FmtResult},
    hash::{Hash, Hasher},
};
use xxhash_rust::xxh3::Xxh3;

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct QueryPlan {
    pub kind: &'static str, // "QueryPlan"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node: Option<PlanNode>,
}

#[allow(clippy::large_enum_variant)]
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "kind")]
pub enum PlanNode {
    Fetch(FetchNode),
    BatchFetch(BatchFetchNode),
    Sequence(SequenceNode),
    Parallel(ParallelNode),
    Flatten(FlattenNode),
    Condition(ConditionNode),
    Subscription(SubscriptionNode),
    Defer(DeferNode),
}

impl PlanNode {
    pub fn as_fetch(&self) -> Option<&FetchNode> {
        match self {
            PlanNode::Fetch(node) => Some(node),
            _ => None,
        }
    }

    pub fn as_batch_fetch(&self) -> Option<&BatchFetchNode> {
        match self {
            PlanNode::BatchFetch(node) => Some(node),
            _ => None,
        }
    }

    pub fn as_sequence(&self) -> Option<&SequenceNode> {
        match self {
            PlanNode::Sequence(node) => Some(node),
            _ => None,
        }
    }

    pub fn as_parallel(&self) -> Option<&ParallelNode> {
        match self {
            PlanNode::Parallel(node) => Some(node),
            _ => None,
        }
    }

    pub fn as_flatten(&self) -> Option<&FlattenNode> {
        match self {
            PlanNode::Flatten(node) => Some(node),
            _ => None,
        }
    }

    pub fn as_condition(&self) -> Option<&ConditionNode> {
        match self {
            PlanNode::Condition(node) => Some(node),
            _ => None,
        }
    }

    pub fn as_subscription(&self) -> Option<&SubscriptionNode> {
        match self {
            PlanNode::Subscription(node) => Some(node),
            _ => None,
        }
    }

    pub fn as_defer(&self) -> Option<&DeferNode> {
        match self {
            PlanNode::Defer(node) => Some(node),
            _ => None,
        }
    }
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
pub struct BatchFetchNode {
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
    pub entity_batch: EntityBatch,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntityBatch {
    pub aliases: Vec<EntityBatchAlias>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntityBatchAlias {
    pub alias: String,
    pub representations_variable_name: String,
    #[serde(rename = "paths")]
    pub merge_paths: Vec<FlattenNodePath>,
    pub requires: SelectionSet,
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

impl ConditionNode {
    /// Checks if this condition node can be merged with another.
    pub fn can_merge_with(&self, other: &Self) -> bool {
        if self.condition != other.condition {
            return false;
        }

        let both_if = self.else_clause.is_none() && other.else_clause.is_none();
        let both_else = self.if_clause.is_none() && other.if_clause.is_none();

        both_if || both_else
    }

    /// Merges another compatible condition node into this one.
    pub fn merge(&mut self, mut other: Self) {
        let merge_into_if_clause = self.if_clause.is_some();
        let mut nodes = self.take_inner_nodes();
        nodes.extend(other.take_inner_nodes());

        let merged_body = PlanNode::sequence(nodes);

        if merge_into_if_clause {
            self.if_clause = Some(Box::new(merged_body));
        } else {
            self.else_clause = Some(Box::new(merged_body));
        }
    }

    fn take_inner_nodes(&mut self) -> Vec<PlanNode> {
        self.if_clause
            .take()
            .or_else(|| self.else_clause.take())
            .map(|n| n.flatten_sequence())
            .unwrap_or_default()
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub struct KeyRenamer {
    pub path: Vec<FetchNodePathSegment>,
    pub rename_key_to: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
pub enum FetchNodePathSegment {
    Key(String),
    TypenameEquals(BTreeSet<String>),
}

impl FetchNodePathSegment {
    pub fn typename_equals_from_type(type_name: String) -> Self {
        Self::TypenameEquals(BTreeSet::from_iter([type_name]))
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
pub enum FetchRewrite {
    ValueSetter(ValueSetter),
    KeyRenamer(KeyRenamer),
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
pub enum FlattenNodePathSegment {
    Field(String),
    TypeCondition(BTreeSet<String>),
    #[serde(rename = "@")]
    List,
}

impl From<&MergePath> for Vec<FetchNodePathSegment> {
    fn from(value: &MergePath) -> Self {
        value
            .inner
            .iter()
            .filter_map(|path_segment| match path_segment {
                Segment::TypeCondition(type_names, _) => {
                    Some(FetchNodePathSegment::TypenameEquals(type_names.clone()))
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

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
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
            FlattenNodePathSegment::TypeCondition(type_names) => {
                write!(
                    f,
                    "|[{}]",
                    type_names.iter().cloned().collect::<Vec<_>>().join("|")
                )
            }
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
                    FlattenNodePathSegment::TypeCondition(_) => {
                        // Don't add a dot before TypeCondition
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
                    Segment::TypeCondition(type_names, _) => {
                        FlattenNodePathSegment::TypeCondition(type_names.clone())
                    }
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

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
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

    /// If the node is a Sequence, returns its children. Otherwise, returns the node itself in a Vec.
    /// This is used to "splice" nodes into a parent Sequence without creating nested Sequences.
    pub fn flatten_sequence(self) -> Vec<PlanNode> {
        match self {
            PlanNode::Sequence(node) => node.nodes,
            other => vec![other],
        }
    }

    /// Flattens nested Parallel nodes into a single list of nodes.
    pub fn flatten_parallel(nodes: Vec<PlanNode>) -> Vec<PlanNode> {
        let mut flattened = Vec::with_capacity(nodes.len());
        for node in nodes {
            match node {
                PlanNode::Parallel(p) => flattened.extend(p.nodes),
                other => flattened.push(other),
            }
        }
        flattened
    }

    pub fn sequence(mut nodes: Vec<PlanNode>) -> PlanNode {
        if nodes.len() == 1 {
            nodes.remove(0)
        } else {
            PlanNode::Sequence(SequenceNode { nodes })
        }
    }

    pub fn parallel(mut nodes: Vec<PlanNode>) -> PlanNode {
        if nodes.len() == 1 {
            nodes.remove(0)
        } else {
            PlanNode::Parallel(ParallelNode { nodes })
        }
    }

    pub fn is_fetching_node(&self) -> bool {
        match self {
            PlanNode::Fetch(_) | PlanNode::BatchFetch(_) => true,
            PlanNode::Flatten(flatten_node) => {
                matches!(flatten_node.node.as_ref(), PlanNode::Fetch(_))
            }
            _ => false,
        }
    }
}

fn create_input_selection_set(
    input_selections: &FetchStepSelections<MultiTypeFetchStep>,
) -> SelectionSet {
    let selection_set: SelectionSet = input_selections.into();

    selection_set.strip_for_plan_input()
}

fn create_output_operation(
    step: &FetchStepData<MultiTypeFetchStep>,
    supergraph: &SupergraphState,
) -> SubgraphFetchOperation {
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
                selections: (&step.output).into(),
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
    let hash = hash_minified_query(&document_str);

    SubgraphFetchOperation {
        document,
        document_str,
        hash,
    }
}

impl From<&FetchStepData<MultiTypeFetchStep>> for OperationKind {
    fn from(step: &FetchStepData<MultiTypeFetchStep>) -> Self {
        match step.input.iter().next().unwrap().0.as_str() {
            "Query" => OperationKind::Query,
            "Mutation" => OperationKind::Mutation,
            "Subscription" => OperationKind::Subscription,
            _ => OperationKind::Query,
        }
    }
}

impl FetchNode {
    pub fn from_fetch_step(
        step: &FetchStepData<MultiTypeFetchStep>,
        supergraph: &SupergraphState,
    ) -> Self {
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
                    selection_set: (&step.output).into(),
                    variable_definitions: step.variable_definitions.clone(),
                };
                let document =
                    minify_operation(operation_def, supergraph).expect("Failed to minify");
                let document_str = document.to_string();
                let hash = hash_minified_query(&document_str);

                FetchNode {
                    id: step.id,
                    service_name: step.service_name.0.clone(),
                    variable_usages: step.variable_usages.clone(),
                    operation_kind: Some(step.into()),
                    operation_name: None,
                    operation: SubgraphFetchOperation {
                        document,
                        document_str,
                        hash,
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
    pub fn from_fetch_step(
        step: &FetchStepData<MultiTypeFetchStep>,
        supergraph: &SupergraphState,
    ) -> Self {
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
                Condition::SkipAndInclude { skip, include } => {
                    let include_node = PlanNode::Condition(ConditionNode {
                        condition: include.clone(),
                        if_clause: Some(Box::new(node)),
                        else_clause: None,
                    });
                    PlanNode::Condition(ConditionNode {
                        condition: skip.clone(),
                        if_clause: None,
                        else_clause: Some(Box::new(include_node)),
                    })
                }
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

impl Display for BatchFetchNode {
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

impl PrettyDisplay for BatchFetchNode {
    fn pretty_fmt(&self, f: &mut FmtFormatter<'_>, depth: usize) -> FmtResult {
        let indent = get_indent(depth);
        writeln!(
            f,
            "{indent}BatchFetch(service: \"{}\") {{",
            self.service_name
        )?;
        writeln!(f, "{indent}  {{")?;
        for alias in &self.entity_batch.aliases {
            writeln!(f, "{indent}    {} {{", alias.alias)?;
            writeln!(f, "{indent}      paths: [")?;
            for merge_path in &alias.merge_paths {
                writeln!(f, "{indent}        \"{}\"", merge_path)?;
            }
            writeln!(f, "{indent}      ]")?;
            writeln!(f, "{indent}      {{")?;
            alias.requires.pretty_fmt(f, depth + 4)?;
            writeln!(f, "{indent}      }}")?;
            writeln!(f, "{indent}    }}")?;
        }
        writeln!(f, "{indent}  }}")?;
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
            PlanNode::BatchFetch(node) => node.pretty_fmt(f, depth),
            PlanNode::Flatten(node) => node.pretty_fmt(f, depth),
            PlanNode::Sequence(node) => node.pretty_fmt(f, depth),
            PlanNode::Parallel(node) => node.pretty_fmt(f, depth),
            PlanNode::Condition(node) => node.pretty_fmt(f, depth),
            _ => Ok(()),
        }
    }
}

pub fn hash_minified_query(minified_query: &str) -> u64 {
    let mut hasher = Xxh3::new();
    minified_query.hash(&mut hasher);
    hasher.finish()
}
