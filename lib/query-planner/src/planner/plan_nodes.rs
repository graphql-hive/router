use crate::graph::selection::SelectionNode;

use super::OperationType;

pub struct Plan {
    pub root: PlanNode,
}

pub enum PlanNode {
    Sequence(Vec<PlanNode>),
    Parallel(Vec<PlanNode>),
    Fetch(FetchNode),
    Flatten(FlattenNode),
}

pub struct FetchNode {
    pub service_name: String,
    pub operation: String,
    pub operation_type: OperationType,
    pub requires: Option<SelectionNode>,
}

pub enum FlattenNodePathType {
    Named(String),
    Indexed(usize),
}

pub struct FlattenNode {
    pub path: Vec<FlattenNodePathType>,
    pub node: Box<PlanNode>,
}
