//! `graphql_tools::validation::rules::ProvidedRequiredArguments`, flat.

use graphql_tools::flat::{Directive, Selection, Span};
use graphql_tools::static_graphql::schema::InputValue;

use crate::pipeline::validation::flat::visit::{
    error, pos, BoxedVisitor, Ctx, Errors, FlatRule, Visitor,
};

const CODE: &str = "ProvidedRequiredArguments";

pub struct ProvidedRequiredArguments;

impl FlatRule for ProvidedRequiredArguments {
    fn visitor<'a>(&self) -> BoxedVisitor<'a> {
        Box::new(ProvidedRequiredArguments)
    }

    fn error_code(&self) -> &'static str {
        CODE
    }
}

/// Required arguments the call didn't pass, in definition order.
fn missing<'a>(
    ctx: &'a Ctx<'_>,
    used: Span,
    defined: &'a [InputValue],
) -> impl Iterator<Item = &'a InputValue> {
    let used = ctx.doc.args_of(used);
    defined
        .iter()
        .filter(move |def| def.is_required() && !used.iter().any(|a| ctx.str(a.name) == def.name))
}

impl<'a> Visitor<'a> for ProvidedRequiredArguments {
    fn enter_field(&mut self, ctx: &mut Ctx<'a>, errors: &mut Errors, field: &'a Selection) {
        let name = ctx.str(field.name);
        let Some(field_def) = ctx
            .current_parent_type()
            .and_then(|t| t.field_by_name(name))
        else {
            return;
        };
        for arg in missing(ctx, field.args, &field_def.arguments) {
            errors.report_error(error(
                CODE,
                format!(
                    "Field \"{}\" argument \"{}\" of type \"{}\" is required, but it was not provided.",
                    name, arg.name, arg.value_type
                ),
                vec![pos(field.pos)],
            ));
        }
    }

    fn enter_directive(&mut self, ctx: &mut Ctx<'a>, errors: &mut Errors, d: &'a Directive) {
        let name = ctx.str(d.name);
        let Some(def) = ctx.directives.get(name).copied() else {
            return;
        };
        for arg in missing(ctx, d.args, &def.arguments) {
            errors.report_error(error(
                CODE,
                format!(
                    "Directive \"@{}\" argument \"{}\" of type \"{}\" is required, but it was not provided.",
                    name, arg.name, arg.value_type
                ),
                vec![pos(d.pos)],
            ));
        }
    }
}
