//! `graphql_tools::validation::rules::LoneAnonymousOperation`, flat.

use graphql_tools::flat::{OperationKind, NONE};

use crate::pipeline::validation::flat::visit::{
    error, pos, BoxedVisitor, Ctx, Errors, FlatRule, Visitor,
};

const CODE: &str = "LoneAnonymousOperation";

pub struct LoneAnonymousOperation;

impl FlatRule for LoneAnonymousOperation {
    fn visitor<'a>(&self) -> BoxedVisitor<'a> {
        Box::new(LoneAnonymousOperation)
    }

    fn error_code(&self) -> &'static str {
        CODE
    }
}

impl<'a> Visitor<'a> for LoneAnonymousOperation {
    fn enter_document(&mut self, ctx: &mut Ctx<'a>, errors: &mut Errors) {
        if ctx.doc.operations().count() < 2 {
            return;
        }
        for op in ctx.doc.operations() {
            if op.name != NONE {
                continue;
            }
            // the tree has no position for `{ ... }`
            let locations = if op.kind == OperationKind::Shorthand {
                vec![]
            } else {
                vec![pos(op.pos)]
            };
            errors.report_error(error(
                CODE,
                "This anonymous operation must be the only defined operation.".to_string(),
                locations,
            ));
        }
    }
}
