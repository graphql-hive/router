//! `graphql_tools::validation::rules::PossibleFragmentSpreads`, flat.

use graphql_tools::flat::Selection;
use graphql_tools::validation::rules::possible_fragment_spreads::do_types_overlap;

use crate::pipeline::validation::flat::visit::{
    error, BoxedVisitor, Ctx, Errors, FlatRule, Visitor,
};

const CODE: &str = "PossibleFragmentSpreads";

pub struct PossibleFragmentSpreads;

impl FlatRule for PossibleFragmentSpreads {
    fn visitor<'a>(&self) -> BoxedVisitor<'a> {
        Box::new(PossibleFragmentSpreads)
    }

    fn error_code(&self) -> &'static str {
        CODE
    }
}

impl<'a> Visitor<'a> for PossibleFragmentSpreads {
    fn enter_inline_fragment(&mut self, ctx: &mut Ctx<'a>, errors: &mut Errors, _: &'a Selection) {
        let (Some(frag_type), Some(parent_type)) = (ctx.current_type(), ctx.current_parent_type())
        else {
            return;
        };
        if frag_type.is_composite_type()
            && parent_type.is_composite_type()
            && !do_types_overlap(ctx.schema, frag_type, parent_type)
        {
            errors.report_error(error(
                CODE,
                format!(
                    "Fragment cannot be spread here as objects of type \"{}\" can never be of type \"{}\".",
                    parent_type.name(),
                    frag_type.name()
                ),
                vec![],
            ));
        }
    }

    fn enter_fragment_spread(
        &mut self,
        ctx: &mut Ctx<'a>,
        errors: &mut Errors,
        spread: &'a Selection,
    ) {
        let Some(fragment) = ctx.known_fragments.get(ctx.str(spread.name)) else {
            return;
        };
        let type_name = ctx.str(fragment.type_condition);
        let (Some(frag_type), Some(parent_type)) =
            (ctx.type_by_name(type_name), ctx.current_parent_type())
        else {
            return;
        };
        if frag_type.is_composite_type()
            && parent_type.is_composite_type()
            && !do_types_overlap(ctx.schema, frag_type, parent_type)
        {
            errors.report_error(error(
                CODE,
                format!(
                    "Fragment \"{}\" cannot be spread here as objects of type \"{}\" can never be of type \"{}\".",
                    ctx.str(fragment.name),
                    parent_type.name(),
                    type_name
                ),
                vec![],
            ));
        }
    }
}
