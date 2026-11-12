//! `graphql_tools::validation::rules::UniqueVariableNames`, flat.

use std::collections::hash_map::Entry;
use std::collections::HashMap;

use graphql_tools::flat::{Operation, Pos, Sym, VariableDefinition};

use crate::pipeline::validation::flat::visit::{
    error, pos, BoxedVisitor, Ctx, Errors, FlatRule, Visitor,
};

const CODE: &str = "UniqueVariableNames";

pub struct UniqueVariableNames;

impl FlatRule for UniqueVariableNames {
    fn visitor<'a>(&self) -> BoxedVisitor<'a> {
        Box::new(Rule {
            found: HashMap::new(),
        })
    }

    fn error_code(&self) -> &'static str {
        CODE
    }
}

struct Rule {
    found: HashMap<Sym, Pos>,
}

impl<'a> Visitor<'a> for Rule {
    fn enter_operation(&mut self, _: &mut Ctx<'a>, _: &mut Errors, _: &'a Operation) {
        self.found.clear();
    }

    fn enter_variable_definition(
        &mut self,
        ctx: &mut Ctx<'a>,
        errors: &mut Errors,
        variable: &'a VariableDefinition,
    ) {
        match self.found.entry(variable.name) {
            Entry::Occupied(first) => errors.report_error(error(
                CODE,
                format!(
                    "There can only be one variable named \"${}\".",
                    ctx.str(variable.name)
                ),
                vec![pos(*first.get()), pos(variable.pos)],
            )),
            Entry::Vacant(slot) => {
                slot.insert(variable.pos);
            }
        }
    }
}
