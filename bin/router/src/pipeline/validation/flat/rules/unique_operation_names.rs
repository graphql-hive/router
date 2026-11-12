//! `graphql_tools::validation::rules::UniqueOperationNames`, flat.

use graphql_tools::flat::{Operation, Sym, NONE};

use crate::pipeline::validation::flat::visit::{
    error, BoxedVisitor, Ctx, Errors, FlatRule, Visitor,
};

const CODE: &str = "UniqueOperationNames";

pub struct UniqueOperationNames;

impl FlatRule for UniqueOperationNames {
    fn visitor<'a>(&self) -> BoxedVisitor<'a> {
        Box::new(Rule { seen: vec![] })
    }

    fn error_code(&self) -> &'static str {
        CODE
    }
}

/// Names with how many times we saw them, in first-seen order. The tree walks a HashMap here,
/// so its order is random anyway; ours is at least stable.
struct Rule {
    seen: Vec<(Sym, u32)>,
}

impl<'a> Visitor<'a> for Rule {
    fn enter_operation(&mut self, _: &mut Ctx<'a>, _: &mut Errors, op: &'a Operation) {
        if op.name == NONE {
            return;
        }
        match self.seen.iter_mut().find(|(name, _)| *name == op.name) {
            Some((_, count)) => *count += 1,
            None => self.seen.push((op.name, 1)),
        }
    }

    fn leave_document(&mut self, ctx: &mut Ctx<'a>, errors: &mut Errors) {
        for &(name, count) in &self.seen {
            if count > 1 {
                errors.report_error(error(
                    CODE,
                    format!(
                        "There can be only one operation named \"{}\".",
                        ctx.str(name)
                    ),
                    vec![],
                ));
            }
        }
    }
}
