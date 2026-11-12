//! `graphql_tools::validation::rules::UniqueArgumentNames`, flat.

use graphql_tools::flat::{Directive, Pos, Selection, Span};

use crate::pipeline::validation::flat::visit::{
    error, pos, BoxedVisitor, Ctx, Errors, FlatRule, Visitor,
};

const CODE: &str = "UniqueArgumentNames";

pub struct UniqueArgumentNames;

impl FlatRule for UniqueArgumentNames {
    fn visitor<'a>(&self) -> BoxedVisitor<'a> {
        Box::new(UniqueArgumentNames)
    }

    fn error_code(&self) -> &'static str {
        CODE
    }
}

/// The tree groups by name in a HashMap, so its order across names is random. We go by the
/// first time a name shows up. Every location is the field's (or directive's), like the tree's.
fn check(ctx: &Ctx<'_>, errors: &mut Errors, at: Pos, args: Span) {
    let args = ctx.doc.args_of(args);
    for (i, arg) in args.iter().enumerate() {
        if args[..i].iter().any(|a| a.name == arg.name) {
            continue;
        }
        let count = args[i..].iter().filter(|a| a.name == arg.name).count();
        if count > 1 {
            errors.report_error(error(
                CODE,
                format!(
                    "There can be only one argument named \"{}\".",
                    ctx.str(arg.name)
                ),
                vec![pos(at); count],
            ));
        }
    }
}

impl<'a> Visitor<'a> for UniqueArgumentNames {
    fn enter_field(&mut self, ctx: &mut Ctx<'a>, errors: &mut Errors, field: &'a Selection) {
        check(ctx, errors, field.pos, field.args);
    }

    fn enter_directive(&mut self, ctx: &mut Ctx<'a>, errors: &mut Errors, d: &'a Directive) {
        check(ctx, errors, d.pos, d.args);
    }
}
