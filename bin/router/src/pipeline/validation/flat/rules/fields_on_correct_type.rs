//! `graphql_tools::validation::rules::FieldsOnCorrectType`, flat.

use graphql_tools::flat::{Kind, Operation, OperationKind, Selection};

use crate::pipeline::validation::flat::visit::{
    error, pos, BoxedVisitor, Ctx, Errors, FlatRule, Visitor,
};

const CODE: &str = "FieldsOnCorrectType";

pub struct FieldsOnCorrectType;

impl FlatRule for FieldsOnCorrectType {
    fn visitor<'a>(&self) -> BoxedVisitor<'a> {
        Box::new(FieldsOnCorrectType)
    }

    fn error_code(&self) -> &'static str {
        CODE
    }
}

impl<'a> Visitor<'a> for FieldsOnCorrectType {
    fn enter_operation(&mut self, ctx: &mut Ctx<'a>, errors: &mut Errors, op: &'a Operation) {
        if op.kind != OperationKind::Subscription {
            return;
        }
        let doc = ctx.doc;
        for &id in doc.items(op.selection_set) {
            let s = doc.selection(id);
            if s.kind == Kind::Field && doc.str(s.name) == "__typename" {
                errors.report_error(error(
                    CODE,
                    "`__typename` may not be included as a root field in a subscription operation"
                        .to_string(),
                    vec![pos(op.pos)],
                ));
            }
        }
    }

    fn enter_field(&mut self, ctx: &mut Ctx<'a>, errors: &mut Errors, field: &'a Selection) {
        let Some(parent_type) = ctx.current_parent_type() else {
            return;
        };
        let name = ctx.str(field.name);
        if name.starts_with("__") {
            return;
        }
        if parent_type.field_by_name(name).is_none() {
            errors.report_error(error(
                CODE,
                format!(
                    "Cannot query field \"{}\" on type \"{}\".",
                    name,
                    parent_type.name()
                ),
                vec![pos(field.pos)],
            ));
        }
    }
}
