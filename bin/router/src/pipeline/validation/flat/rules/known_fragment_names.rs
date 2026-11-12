//! `graphql_tools::validation::rules::KnownFragmentNames`, flat.

use graphql_tools::flat::Selection;

use crate::pipeline::validation::flat::visit::{
    error, pos, BoxedVisitor, Ctx, Errors, FlatRule, Visitor,
};

const CODE: &str = "KnownFragmentNames";

pub struct KnownFragmentNames;

impl FlatRule for KnownFragmentNames {
    fn visitor<'a>(&self) -> BoxedVisitor<'a> {
        Box::new(KnownFragmentNames)
    }

    fn error_code(&self) -> &'static str {
        CODE
    }
}

impl<'a> Visitor<'a> for KnownFragmentNames {
    fn enter_fragment_spread(
        &mut self,
        ctx: &mut Ctx<'a>,
        errors: &mut Errors,
        spread: &'a Selection,
    ) {
        let name = ctx.str(spread.name);
        if !ctx.known_fragments.contains_key(name) {
            errors.report_error(error(
                CODE,
                format!("Unknown fragment \"{}\".", name),
                vec![pos(spread.pos)],
            ));
        }
    }
}
