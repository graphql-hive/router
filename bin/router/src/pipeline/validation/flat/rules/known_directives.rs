//! `graphql_tools::validation::rules::KnownDirectives`, flat.

use graphql_tools::flat::{Directive, Fragment, Operation, OperationKind, Selection};
use graphql_tools::static_graphql::schema::DirectiveLocation;

use crate::pipeline::validation::flat::visit::{
    error, pos, BoxedVisitor, Ctx, Errors, FlatRule, Visitor,
};

const CODE: &str = "KnownDirectives";

pub struct KnownDirectives;

impl FlatRule for KnownDirectives {
    fn visitor<'a>(&self) -> BoxedVisitor<'a> {
        Box::new(Rule { location: None })
    }

    fn error_code(&self) -> &'static str {
        CODE
    }
}

/// Same as the tree's: set on enter, cleared on leave, even when that leaves a parent's
/// location behind (it doesn't matter, directives come before the children).
struct Rule {
    location: Option<DirectiveLocation>,
}

impl<'a> Visitor<'a> for Rule {
    fn enter_operation(&mut self, _: &mut Ctx<'a>, _: &mut Errors, op: &'a Operation) {
        self.location = Some(match op.kind {
            OperationKind::Query | OperationKind::Shorthand => DirectiveLocation::Query,
            OperationKind::Mutation => DirectiveLocation::Mutation,
            OperationKind::Subscription => DirectiveLocation::Subscription,
        });
    }

    fn leave_operation(&mut self, _: &mut Ctx<'a>, _: &mut Errors, _: &'a Operation) {
        self.location = None;
    }

    fn enter_field(&mut self, _: &mut Ctx<'a>, _: &mut Errors, _: &'a Selection) {
        self.location = Some(DirectiveLocation::Field);
    }

    fn leave_field(&mut self, _: &mut Ctx<'a>, _: &mut Errors, _: &'a Selection) {
        self.location = None;
    }

    fn enter_fragment_definition(&mut self, _: &mut Ctx<'a>, _: &mut Errors, _: &'a Fragment) {
        self.location = Some(DirectiveLocation::FragmentDefinition);
    }

    fn leave_fragment_definition(&mut self, _: &mut Ctx<'a>, _: &mut Errors, _: &'a Fragment) {
        self.location = None;
    }

    fn enter_fragment_spread(&mut self, _: &mut Ctx<'a>, _: &mut Errors, _: &'a Selection) {
        self.location = Some(DirectiveLocation::FragmentSpread);
    }

    fn leave_fragment_spread(&mut self, _: &mut Ctx<'a>, _: &mut Errors, _: &'a Selection) {
        self.location = None;
    }

    fn enter_inline_fragment(&mut self, _: &mut Ctx<'a>, _: &mut Errors, _: &'a Selection) {
        self.location = Some(DirectiveLocation::InlineFragment);
    }

    fn leave_inline_fragment(&mut self, _: &mut Ctx<'a>, _: &mut Errors, _: &'a Selection) {
        self.location = None;
    }

    fn enter_directive(&mut self, ctx: &mut Ctx<'a>, errors: &mut Errors, d: &'a Directive) {
        let name = ctx.str(d.name);
        let Some(def) = ctx.directives.get(name) else {
            errors.report_error(error(
                CODE,
                format!("Unknown directive \"@{}\".", name),
                vec![pos(d.pos)],
            ));
            return;
        };
        if let Some(location) = &self.location {
            if !def.locations.contains(location) {
                errors.report_error(error(
                    CODE,
                    format!(
                        "Directive \"@{}\" may not be used on {}",
                        name,
                        location.as_str()
                    ),
                    vec![pos(d.pos)],
                ));
            }
        }
    }
}
