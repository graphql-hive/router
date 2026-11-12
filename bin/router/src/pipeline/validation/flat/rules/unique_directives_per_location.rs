//! `graphql_tools::validation::rules::UniqueDirectivesPerLocation`, flat.

use graphql_tools::flat::{Fragment, Operation, Selection, Span};

use crate::pipeline::validation::flat::visit::{
    error, pos, BoxedVisitor, Ctx, Errors, FlatRule, Visitor,
};

const CODE: &str = "UniqueDirectivesPerLocation";

pub struct UniqueDirectivesPerLocation;

impl FlatRule for UniqueDirectivesPerLocation {
    fn visitor<'a>(&self) -> BoxedVisitor<'a> {
        Box::new(UniqueDirectivesPerLocation)
    }

    fn error_code(&self) -> &'static str {
        CODE
    }
}

fn check(ctx: &Ctx<'_>, errors: &mut Errors, directives: Span) {
    let directives = ctx.doc.directives_of(directives);
    let mut seen = Vec::new();
    for directive in directives {
        let name = ctx.str(directive.name);
        let Some(def) = ctx.directives.get(name) else {
            continue;
        };
        if def.repeatable {
            continue;
        }
        if seen.contains(&directive.name) {
            errors.report_error(error(
                CODE,
                format!("Duplicate directive \"{}\"", name),
                vec![pos(directive.pos)],
            ));
            continue;
        }
        seen.push(directive.name);
    }
}

impl<'a> Visitor<'a> for UniqueDirectivesPerLocation {
    fn enter_operation(&mut self, ctx: &mut Ctx<'a>, errors: &mut Errors, op: &'a Operation) {
        check(ctx, errors, op.directives);
    }

    fn enter_field(&mut self, ctx: &mut Ctx<'a>, errors: &mut Errors, field: &'a Selection) {
        check(ctx, errors, field.directives);
    }

    fn enter_fragment_definition(
        &mut self,
        ctx: &mut Ctx<'a>,
        errors: &mut Errors,
        fragment: &'a Fragment,
    ) {
        check(ctx, errors, fragment.directives);
    }

    fn enter_fragment_spread(&mut self, ctx: &mut Ctx<'a>, errors: &mut Errors, s: &'a Selection) {
        check(ctx, errors, s.directives);
    }

    fn enter_inline_fragment(&mut self, ctx: &mut Ctx<'a>, errors: &mut Errors, s: &'a Selection) {
        check(ctx, errors, s.directives);
    }
}
