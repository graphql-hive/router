//! `graphql_tools::validation::rules::FragmentsOnCompositeTypes`, flat.

use graphql_tools::flat::{Fragment, Selection};

use crate::pipeline::validation::flat::visit::{
    error, pos, BoxedVisitor, Ctx, Errors, FlatRule, Visitor,
};

const CODE: &str = "FragmentsOnCompositeTypes";

pub struct FragmentsOnCompositeTypes;

impl FlatRule for FragmentsOnCompositeTypes {
    fn visitor<'a>(&self) -> BoxedVisitor<'a> {
        Box::new(FragmentsOnCompositeTypes)
    }

    fn error_code(&self) -> &'static str {
        CODE
    }
}

impl<'a> Visitor<'a> for FragmentsOnCompositeTypes {
    fn enter_inline_fragment(&mut self, ctx: &mut Ctx<'a>, errors: &mut Errors, f: &'a Selection) {
        let Some(name) = ctx.doc.opt_str(f.type_condition) else {
            return;
        };
        if ctx
            .type_by_name(name)
            .is_some_and(|t| !t.is_composite_type())
        {
            errors.report_error(error(
                CODE,
                format!(
                    "Fragment cannot condition on non composite type \"{}\".",
                    name
                ),
                vec![pos(f.pos)],
            ));
        }
    }

    fn enter_fragment_definition(
        &mut self,
        ctx: &mut Ctx<'a>,
        errors: &mut Errors,
        f: &'a Fragment,
    ) {
        let name = ctx.str(f.type_condition);
        if ctx
            .type_by_name(name)
            .is_some_and(|t| !t.is_composite_type())
        {
            errors.report_error(error(
                CODE,
                format!(
                    "Fragment \"{}\" cannot condition on non composite type \"{}\".",
                    ctx.str(f.name),
                    name
                ),
                vec![pos(f.pos)],
            ));
        }
    }
}
