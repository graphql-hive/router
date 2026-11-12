//! `graphql_tools::validation::rules::KnownArgumentNames`, flat.

use graphql_tools::flat::{Argument, Directive, Selection};
use graphql_tools::static_graphql::schema::{InputValue, TypeDefinition};

use crate::pipeline::validation::flat::visit::{
    error, BoxedVisitor, Ctx, Errors, FlatRule, Visitor,
};

const CODE: &str = "KnownArgumentNames";

pub struct KnownArgumentNames;

impl FlatRule for KnownArgumentNames {
    fn visitor<'a>(&self) -> BoxedVisitor<'a> {
        Box::new(Rule { current: None })
    }

    fn error_code(&self) -> &'static str {
        CODE
    }
}

enum Parent<'a> {
    Field(&'a str, &'a TypeDefinition),
    Directive(&'a str),
}

struct Rule<'a> {
    current: Option<(Parent<'a>, &'a Vec<InputValue>)>,
}

impl<'a> Visitor<'a> for Rule<'a> {
    fn enter_directive(&mut self, ctx: &mut Ctx<'a>, _: &mut Errors, directive: &'a Directive) {
        if let Some(def) = ctx.directive_by_name(ctx.str(directive.name)) {
            self.current = Some((Parent::Directive(&def.name), &def.arguments));
        }
    }

    fn leave_directive(&mut self, _: &mut Ctx<'a>, _: &mut Errors, _: &'a Directive) {
        self.current = None;
    }

    fn enter_field(&mut self, ctx: &mut Ctx<'a>, _: &mut Errors, field: &'a Selection) {
        if let Some(parent_type) = ctx.current_parent_type() {
            if let Some(field_def) = parent_type.field_by_name(ctx.str(field.name)) {
                self.current = Some((
                    Parent::Field(&field_def.name, parent_type),
                    &field_def.arguments,
                ));
            }
        }
    }

    fn leave_field(&mut self, _: &mut Ctx<'a>, _: &mut Errors, _: &'a Selection) {
        self.current = None;
    }

    fn enter_argument(&mut self, ctx: &mut Ctx<'a>, errors: &mut Errors, argument: &'a Argument) {
        let Some((parent, args)) = &self.current else {
            return;
        };
        let name = ctx.str(argument.name);
        if args.iter().any(|a| a.name == name) {
            return;
        }
        let message = match parent {
            Parent::Field(field_name, type_def) => format!(
                "Unknown argument \"{}\" on field \"{}.{}\".",
                name,
                type_def.name(),
                field_name
            ),
            Parent::Directive(directive_name) => format!(
                "Unknown argument \"{}\" on directive \"@{}\".",
                name, directive_name
            ),
        };
        errors.report_error(error(CODE, message, vec![]));
    }
}
