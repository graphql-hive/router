//! `graphql_tools::validation::rules::KnownTypeNames`, flat.

use graphql_tools::flat::{Fragment, Pos, Selection, VariableDefinition};

use crate::pipeline::validation::flat::visit::{
    error, pos, BoxedVisitor, Ctx, Errors, FlatRule, Visitor,
};

const CODE: &str = "KnownTypeNames";

pub struct KnownTypeNames;

impl FlatRule for KnownTypeNames {
    fn visitor<'a>(&self) -> BoxedVisitor<'a> {
        Box::new(KnownTypeNames)
    }

    fn error_code(&self) -> &'static str {
        CODE
    }
}

fn check(ctx: &Ctx, errors: &mut Errors, name: &str, at: Pos) {
    if ctx.type_by_name(name).is_none() && !name.starts_with("__") {
        errors.report_error(error(
            CODE,
            format!("Unknown type \"{}\".", name),
            vec![pos(at)],
        ));
    }
}

impl<'a> Visitor<'a> for KnownTypeNames {
    fn enter_fragment_definition(
        &mut self,
        ctx: &mut Ctx<'a>,
        errors: &mut Errors,
        f: &'a Fragment,
    ) {
        check(ctx, errors, ctx.str(f.type_condition), f.pos);
    }

    fn enter_inline_fragment(&mut self, ctx: &mut Ctx<'a>, errors: &mut Errors, f: &'a Selection) {
        if let Some(name) = ctx.doc.opt_str(f.type_condition) {
            check(ctx, errors, name, f.pos);
        }
    }

    fn enter_variable_definition(
        &mut self,
        ctx: &mut Ctx<'a>,
        errors: &mut Errors,
        variable: &'a VariableDefinition,
    ) {
        check(ctx, errors, ctx.ty(variable.ty).inner_type(), variable.pos);
    }
}
