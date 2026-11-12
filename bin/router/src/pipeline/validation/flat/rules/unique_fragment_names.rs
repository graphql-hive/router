//! `graphql_tools::validation::rules::UniqueFragmentNames`, flat.

use graphql_tools::flat::{Fragment, Sym};

use crate::pipeline::validation::flat::visit::{
    error, BoxedVisitor, Ctx, Errors, FlatRule, Visitor,
};

const CODE: &str = "UniqueFragmentNames";

pub struct UniqueFragmentNames;

impl FlatRule for UniqueFragmentNames {
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
    fn enter_fragment_definition(&mut self, _: &mut Ctx<'a>, _: &mut Errors, f: &'a Fragment) {
        match self.seen.iter_mut().find(|(name, _)| *name == f.name) {
            Some((_, count)) => *count += 1,
            None => self.seen.push((f.name, 1)),
        }
    }

    fn leave_document(&mut self, ctx: &mut Ctx<'a>, errors: &mut Errors) {
        for &(name, count) in &self.seen {
            if count > 1 {
                errors.report_error(error(
                    CODE,
                    format!(
                        "There can be only one fragment named \"{}\".",
                        ctx.str(name)
                    ),
                    vec![],
                ));
            }
        }
    }
}
