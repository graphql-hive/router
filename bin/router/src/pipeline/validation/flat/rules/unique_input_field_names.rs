//! `graphql_tools::validation::rules::UniqueInputFieldNames`, flat.

use graphql_tools::flat::Span;

use crate::pipeline::validation::flat::visit::{
    error, BoxedVisitor, Ctx, Errors, FlatRule, Visitor,
};

const CODE: &str = "UniqueInputFieldNames";

pub struct UniqueInputFieldNames;

impl FlatRule for UniqueInputFieldNames {
    fn visitor<'a>(&self) -> BoxedVisitor<'a> {
        Box::new(UniqueInputFieldNames)
    }

    fn error_code(&self) -> &'static str {
        CODE
    }
}

impl<'a> Visitor<'a> for UniqueInputFieldNames {
    fn enter_object_value(&mut self, ctx: &mut Ctx<'a>, errors: &mut Errors, fields: Span) {
        let fields = &ctx.doc.object_fields[fields.range()];
        for (i, (name, _)) in fields.iter().enumerate() {
            // objects are tiny, a scan beats a set here
            if fields[..i].iter().any(|(seen, _)| seen == name) {
                errors.report_error(error(
                    CODE,
                    format!(
                        "There can be only one input field named \"{}\".",
                        ctx.str(*name)
                    ),
                    vec![],
                ));
            }
        }
    }
}
