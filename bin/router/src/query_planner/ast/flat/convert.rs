//! The planner's `OperationDefinition` from a normalized flat operation, the same as
//! `create_normalized_document` makes from the parser's tree. Its last step, a skipped
//! `__typename` in every composite field left with no selections, is done on the way.

use std::collections::BTreeMap;

use super::normalize::Norm;
use super::{Kind, Operation, OperationKind, Span, TypeRef, Value, ValueId, NONE};
use crate::query_planner::ast::arguments::ArgumentsMap;
use crate::query_planner::ast::document::NormalizedDocument;
use crate::query_planner::ast::operation::{OperationDefinition, VariableDefinition};
use crate::query_planner::ast::selection_item::SelectionItem;
use crate::query_planner::ast::selection_set::{
    FieldSelection, InlineFragmentSelection, SelectionSet,
};
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
    let operation = OperationDefinition {
        name: name.clone(),
        operation_kind: Some(operation_kind),
        selection_set: selection_set(n, op.selection_set, root),
        variable_definitions,
    };
    NormalizedDocument {
        operation,
        operation_name: operation_name.map(str::to_string).or(name),
    }
}

/// `parent` is the type of the selection set, when the `__typename` step goes into it. It doesn't
/// go below a field whose type it can't tell, or isn't composite.
fn selection_set<'a>(
    n: &mut Norm<'a>,
    list: Span,
    parent: Option<&'a SupergraphDefinition>,
) -> SelectionSet {
    let mut items = Vec::with_capacity(list.len as usize);
    for index in list.range() {
        let selection = n.sels[n.kids[index] as usize];
        let (skip_if, include_if) = conditions(n, selection.directives);
        items.push(match selection.kind {
            Kind::Field => {
                let child_parent = parent
                    .and_then(|parent| n.field_info(parent.fields(), selection.name))
                    .and_then(|info| info.ty)
                    .filter(|ty| ty.is_composite_type());
                let mut selections = selection_set(n, selection.children, child_parent);
                if child_parent.is_some() && selections.items.is_empty() {
                    selections
                        .items
                        .push(SelectionItem::Field(FieldSelection::new_skipped_typename()));
                }
                SelectionItem::Field(FieldSelection {
                    name: n.s(selection.name).to_string(),
                    alias: (selection.alias != NONE).then(|| n.s(selection.alias).to_string()),
                    arguments: (!selection.args.is_empty()).then(|| {
                        ArgumentsMap::from(
                            n.args[selection.args.range()]
                                .iter()
                                .map(|arg| (n.s(arg.name).to_string(), value_of(n, arg.value)))
                                .collect::<Vec<_>>(),
                        )
                    }),
                    selections,
                    skip_if,
                    include_if,
                    omit_from_response: false,
                })
            }
            Kind::InlineFragment => {
                let child_parent = parent.and(n.defs[selection.type_condition as usize]);
                SelectionItem::InlineFragment(InlineFragmentSelection {
                    type_condition: n.s(selection.type_condition).to_string(),
                    selections: selection_set(n, selection.children, child_parent),
                    skip_if,
                    include_if,
                })
            }
            Kind::FragmentSpread => SelectionItem::FragmentSpread(n.s(selection.name).into()),
        });
    }
    SelectionSet { items }
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
