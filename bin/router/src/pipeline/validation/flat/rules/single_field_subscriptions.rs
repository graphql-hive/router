//! `graphql_tools::validation::rules::SingleFieldSubscriptions`, flat.

use graphql_tools::flat::{Kind, ListId, Operation, OperationKind, Sym, NONE};
use graphql_tools::static_graphql::schema::{ObjectType, TypeDefinition};

use crate::pipeline::validation::flat::visit::{
    error, pos, BoxedVisitor, Ctx, Errors, FlatRule, Visitor,
};

const CODE: &str = "SingleFieldSubscriptions";

pub struct SingleFieldSubscriptions;

impl FlatRule for SingleFieldSubscriptions {
    fn visitor<'a>(&self) -> BoxedVisitor<'a> {
        Box::new(SingleFieldSubscriptions)
    }

    fn error_code(&self) -> &'static str {
        CODE
    }
}

/// The tree's `does_fragment_condition_match`, against the subscription root.
fn matches(ctx: &Ctx, condition: Sym, root: &ObjectType) -> bool {
    let Some(name) = ctx.doc.opt_str(condition) else {
        return true;
    };
    match ctx.type_by_name(name) {
        Some(t) if t.name() == root.name => true,
        Some(TypeDefinition::Interface(i)) => root.implements_interfaces.contains(&i.name),
        Some(TypeDefinition::Union(u)) => u.types.contains(&root.name),
        _ => false,
    }
}

/// The tree's `collect_fields`, keeping only the distinct field names (not response keys).
fn collect<'a>(
    ctx: &Ctx<'a>,
    list: ListId,
    root: &ObjectType,
    names: &mut Vec<&'a str>,
    visited: &mut Vec<Sym>,
) {
    let doc = ctx.doc;
    for &id in doc.items(list) {
        let s = doc.selection(id);
        match s.kind {
            Kind::Field => {
                let name = ctx.str(s.name);
                if !names.contains(&name) {
                    names.push(name);
                }
            }
            Kind::InlineFragment => {
                if matches(ctx, s.type_condition, root) {
                    collect(ctx, s.children, root, names, visited);
                }
            }
            Kind::FragmentSpread => {
                if visited.contains(&s.name) {
                    continue;
                }
                visited.push(s.name);
                if let Some(f) = ctx.known_fragments.get(ctx.str(s.name)) {
                    if matches(ctx, f.type_condition, root) {
                        collect(ctx, f.selection_set, root, names, visited);
                    }
                }
            }
        }
    }
}

impl<'a> Visitor<'a> for SingleFieldSubscriptions {
    fn enter_operation(&mut self, ctx: &mut Ctx<'a>, errors: &mut Errors, op: &'a Operation) {
        if op.kind != OperationKind::Subscription {
            return;
        }
        let Some(root) = ctx.schema.subscription_type() else {
            return;
        };
        let mut names = vec![];
        collect(ctx, op.selection_set, root, &mut names, &mut vec![]);
        let op_name = (op.name != NONE).then(|| ctx.str(op.name));

        if names.len() > 1 {
            let message = match op_name {
                Some(n) => format!(
                    "Subscription \"{}\" must select only one top level field.",
                    n
                ),
                None => "Anonymous Subscription must select only one top level field.".to_owned(),
            };
            errors.report_error(error(CODE, message, vec![pos(op.pos)]));
        }
        // one per introspection field; all the same, so the tree's HashMap order doesn't matter
        for _ in names.iter().filter(|n| n.starts_with("__")) {
            let message = match op_name {
                Some(n) => format!(
                    "Subscription \"{}\" must not select an introspection top level field.",
                    n
                ),
                None => "Anonymous Subscription must not select an introspection top level field."
                    .to_owned(),
            };
            errors.report_error(error(CODE, message, vec![pos(op.pos)]));
        }
    }
}
