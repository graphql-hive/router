//! `graphql_tools::validation::rules::VariablesAreInputTypes`, flat.

use graphql_tools::flat::VariableDefinition;

use crate::pipeline::validation::flat::visit::{
    error, pos, BoxedVisitor, Ctx, Errors, FlatRule, Visitor,
};

const CODE: &str = "VariablesAreInputTypes";

pub struct VariablesAreInputTypes;

impl FlatRule for VariablesAreInputTypes {
    fn visitor<'a>(&self) -> BoxedVisitor<'a> {
        Box::new(VariablesAreInputTypes)
    }

    fn error_code(&self) -> &'static str {
        CODE
    }
}

impl<'a> Visitor<'a> for VariablesAreInputTypes {
    fn enter_variable_definition(
        &mut self,
        ctx: &mut Ctx<'a>,
        errors: &mut Errors,
        variable: &'a VariableDefinition,
    ) {
        let ty = ctx.ty(variable.ty);
        if ctx
            .type_by_name(ty.inner_type())
            .is_some_and(|t| !t.is_input_type())
        {
            errors.report_error(error(
                CODE,
                format!(
                    "Variable \"${}\" cannot be non-input type \"{}\".",
                    ctx.str(variable.name),
                    ty
                ),
                vec![pos(variable.pos)],
            ));
        }
    }
}
