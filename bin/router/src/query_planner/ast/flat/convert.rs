//! The planner's flat `PlanDocument` and `PlanOperation`, or a field set's `SelectionTree`, from a
//! normalized flat operation. For an operation, every composite field left with no selections
//! gets a skipped `__typename` on the way.

use std::collections::BTreeMap;

use super::arena::{Arena, ListId};
use super::normalize::Norm;
use super::plan_document::PlanDocument;
use super::selection_tree::{PlanOperation, SelectionAttributes, SelectionTree};
use super::{Kind, Operation, OperationKind, Span, TypeRef, Value, ValueId, NONE};
use crate::query_planner::ast::arguments::ArgumentsMap;
use crate::query_planner::ast::document::NormalizedDocument;
use crate::query_planner::ast::merge_path::Condition;
use crate::query_planner::ast::operation::VariableDefinition;
use crate::query_planner::ast::value::Value as AstValue;
use crate::query_planner::state::supergraph_state::{
    OperationKind as AstKind, SupergraphDefinition, TypeNode,
};

pub(super) fn normalized_document<'a>(
    n: &mut Norm<'a>,
    op: Operation,
    operation_name: Option<&str>,
) -> NormalizedDocument {
    let name = (op.name != NONE).then(|| n.s(op.name).to_string());
    let operation_kind = match op.kind {
        OperationKind::Query | OperationKind::Shorthand => AstKind::Query,
        OperationKind::Mutation => AstKind::Mutation,
        OperationKind::Subscription => AstKind::Subscription,
    };
    let doc = n.doc;
    let state = n.state;
    let root = match op.kind {
        OperationKind::Query | OperationKind::Shorthand => Some(state.query_type.as_str()),
        OperationKind::Mutation => state.mutation_type.as_deref(),
        OperationKind::Subscription => state.subscription_type.as_deref(),
    };
    let root = root.and_then(|name| state.definitions.get(name));
    let variable_definitions = (!op.variables.is_empty()).then(|| {
        doc.variables[op.variables.range()]
            .iter()
            .map(|variable| VariableDefinition {
                name: n.s(variable.name).to_string(),
                variable_type: type_node(n, variable.ty),
                default_value: variable.default_value.map(|value| value_of(n, value)),
            })
            .collect()
    });
    let mut arena = Arena::new();
    let selections = arena_list(n, &mut arena, op.selection_set, root);
    let document = PlanDocument {
        arena,
        kind: Some(operation_kind),
        name: name.clone(),
        variables: variable_definitions,
        selections,
        fragments: vec![],
    };
    let plan_operation = PlanOperation::from_document(&document);
    NormalizedDocument {
        operation_name: operation_name.map(str::to_string).or(name),
        plan_operation: std::sync::Arc::new(plan_operation),
        document: std::sync::Arc::new(document),
    }
}

/// `selection_set`, into an arena: the normalized operation the executor and the planner read.
fn arena_list<'a>(
    n: &mut Norm<'a>,
    arena: &mut Arena,
    list: Span,
    parent: Option<&'a SupergraphDefinition>,
) -> ListId {
    let mut ids = Vec::with_capacity(list.len as usize);
    for index in list.range() {
        let selection = n.sels[n.kids[index] as usize];
        let (skip_if, include_if) = conditions(n, selection.directives);
        let condition = match (skip_if, include_if) {
            (Some(skip), Some(include)) => Some(Condition::SkipAndInclude { skip, include }),
            (Some(variable), None) => Some(Condition::Skip(variable)),
            (None, Some(variable)) => Some(Condition::Include(variable)),
            (None, None) => None,
        };
        ids.push(match selection.kind {
            Kind::Field => {
                let child_parent = parent
                    .and_then(|parent| n.field_info(parent.fields(), selection.name))
                    .and_then(|info| info.ty)
                    .filter(|ty| ty.is_composite_type());
                let mut children = arena_list(n, arena, selection.children, child_parent);
                if child_parent.is_some() && arena.is_empty(children) {
                    let typename = arena.new_field("__typename", None, None, None);
                    arena.node_mut(typename).omit_from_response = true;
                    children = arena.list_of(&[typename]);
                }
                let alias = (selection.alias != NONE).then(|| n.s(selection.alias).to_string());
                let arguments =
                    (!selection.args.is_empty()).then(|| arguments_of(n, selection.args));
                let name = n.s(selection.name).to_string();
                let id = arena.new_field(
                    &name,
                    alias.as_deref(),
                    arguments.as_ref(),
                    condition.as_ref(),
                );
                arena.node_mut(id).children = children;
                id
            }
            Kind::InlineFragment => {
                let child_parent = parent.and(n.defs[selection.type_condition as usize]);
                let children = arena_list(n, arena, selection.children, child_parent);
                let name = n.s(selection.type_condition).to_string();
                let id = arena.new_fragment(&name, condition.as_ref());
                arena.node_mut(id).children = children;
                id
            }
            Kind::FragmentSpread => {
                let name = n.s(selection.name).to_string();
                arena.new_spread(&name)
            }
        });
    }
    arena.list_of(&ids)
}

/// A field set, as a `SelectionTree` of its own. No `__typename` step.
pub(super) fn field_set(n: &mut Norm<'_>, list: Span) -> SelectionTree {
    let mut tree = SelectionTree::default();
    let roots = selection_tree(n, &mut tree, list, None);
    tree.set_roots(roots);
    tree
}

/// `selection_set`, into the walker's `SelectionTree`: the same selections, `__typename` step
/// included.
fn selection_tree<'a>(
    n: &mut Norm<'a>,
    tree: &mut SelectionTree,
    list: Span,
    parent: Option<&'a SupergraphDefinition>,
) -> Span {
    let mut ids = Vec::with_capacity(list.len as usize);
    for index in list.range() {
        let selection = n.sels[n.kids[index] as usize];
        let (skip_if, include_if) = conditions(n, selection.directives);
        let condition = match (skip_if, include_if) {
            (Some(skip), Some(include)) => Some(Condition::SkipAndInclude { skip, include }),
            (Some(variable), None) => Some(Condition::Skip(variable)),
            (None, Some(variable)) => Some(Condition::Include(variable)),
            (None, None) => None,
        };
        ids.push(match selection.kind {
            Kind::Field => {
                let child_parent = parent
                    .and_then(|parent| n.field_info(parent.fields(), selection.name))
                    .and_then(|info| info.ty)
                    .filter(|ty| ty.is_composite_type());
                let mut children = selection_tree(n, tree, selection.children, child_parent);
                if child_parent.is_some() && children.is_empty() {
                    let typename =
                        tree.add_node(Kind::Field, "__typename", Span::EMPTY, None, None);
                    children = tree.add_children(&[typename]);
                }
                let alias = (selection.alias != NONE).then(|| n.s(selection.alias).to_string());
                let arguments =
                    (!selection.args.is_empty()).then(|| arguments_of(n, selection.args));
                let attributes = (alias.is_some() || arguments.is_some())
                    .then_some(SelectionAttributes { alias, arguments });
                let name = n.s(selection.name).to_string();
                tree.add_node(Kind::Field, &name, children, condition, attributes)
            }
            Kind::InlineFragment => {
                let child_parent = parent.and(n.defs[selection.type_condition as usize]);
                let children = selection_tree(n, tree, selection.children, child_parent);
                let name = n.s(selection.type_condition).to_string();
                tree.add_node(Kind::InlineFragment, &name, children, condition, None)
            }
            Kind::FragmentSpread => {
                let name = n.s(selection.name).to_string();
                tree.add_node(Kind::FragmentSpread, &name, Span::EMPTY, None, None)
            }
        });
    }
    tree.add_children(&ids)
}

fn arguments_of(n: &Norm<'_>, args: Span) -> ArgumentsMap {
    ArgumentsMap::from(
        n.args[args.range()]
            .iter()
            .map(|arg| (n.s(arg.name).to_string(), value_of(n, arg.value)))
            .collect::<Vec<_>>(),
    )
}

/// The variables of the last `@skip(if: $v)` and `@include(if: $v)`.
fn conditions(n: &Norm<'_>, directives: Span) -> (Option<String>, Option<String>) {
    let mut skip_if = None;
    let mut include_if = None;
    for directive in &n.directives[directives.range()] {
        let target = match n.s(directive.name) {
            "skip" => &mut skip_if,
            "include" => &mut include_if,
            _ => continue,
        };
        let if_arg = n.args[directive.args.range()]
            .iter()
            .find(|arg| n.s(arg.name) == "if");
        if let Some(arg) = if_arg {
            if let Value::Variable(name) = n.doc.values[arg.value as usize] {
                *target = Some(n.s(name).to_string());
            }
        }
    }
    (skip_if, include_if)
}

fn value_of(n: &Norm<'_>, id: ValueId) -> AstValue {
    let doc = n.doc;
    match doc.values[id as usize] {
        Value::Variable(name) => AstValue::Variable(n.s(name).to_string()),
        Value::Int(int) => AstValue::Int(int),
        Value::Float(float) => AstValue::Float(float),
        Value::String(s) => AstValue::String(n.s(s).to_string()),
        Value::Boolean(b) => AstValue::Boolean(b),
        Value::Null => AstValue::Null,
        Value::Enum(name) => AstValue::Enum(n.s(name).to_string()),
        Value::List(items) => AstValue::List(
            doc.value_items[items.range()]
                .iter()
                .map(|item| value_of(n, *item))
                .collect(),
        ),
        Value::Object(fields) => AstValue::Object(
            doc.object_fields[fields.range()]
                .iter()
                .map(|(name, value)| (n.s(*name).to_string(), value_of(n, *value)))
                .collect::<BTreeMap<_, _>>(),
        ),
    }
}

fn type_node(n: &Norm<'_>, id: u32) -> TypeNode {
    match n.doc.types[id as usize] {
        TypeRef::Named(name) => TypeNode::Named(n.s(name).to_string()),
        TypeRef::List(inner) => TypeNode::List(Box::new(type_node(n, inner))),
        TypeRef::NonNull(inner) => TypeNode::NonNull(Box::new(type_node(n, inner))),
    }
}
