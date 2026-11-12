//! `graphql_tools::validation::rules::LeafFieldSelections`, flat.

use graphql_tools::flat::Selection;

use crate::pipeline::validation::flat::visit::{
    error, pos, BoxedVisitor, Ctx, Errors, FlatRule, Visitor,
};

const CODE: &str = "LeafFieldSelections";

pub struct LeafFieldSelections;

impl FlatRule for LeafFieldSelections {
    fn visitor<'a>(&self) -> BoxedVisitor<'a> {
        Box::new(LeafFieldSelections)
    }

    fn error_code(&self) -> &'static str {
        CODE
    }
}

impl<'a> Visitor<'a> for LeafFieldSelections {
    fn enter_field(&mut self, ctx: &mut Ctx<'a>, errors: &mut Errors, field: &'a Selection) {
        let (Some(field_type), Some(literal)) = (ctx.current_type(), ctx.current_type_literal())
        else {
            return;
        };
        let name = ctx.str(field.name);
        let has_selection = !ctx.doc.is_empty(field.children);
        if field_type.is_leaf_type() {
            if has_selection {
                errors.report_error(error(
                    CODE,
                    format!(
                        "Field \"{}\" must not have a selection since type \"{}\" has no subfields.",
                        name, literal
                    ),
                    vec![pos(field.pos)],
                ));
            }
        } else if !has_selection {
            errors.report_error(error(
                CODE,
                format!(
                    "Field \"{}\" of type \"{}\" must have a selection of subfields. Did you mean \"{} {{ ... }}\"?",
                    name, literal, name
                ),
                vec![pos(field.pos)],
            ));
        }
    }
}
