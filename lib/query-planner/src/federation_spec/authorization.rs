use crate::ast::value::Value as AstValue;
use graphql_tools::parser::schema::Directive;

use super::directives::FederationDirective;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct AuthenticatedDirective {}

impl AuthenticatedDirective {
    pub const NAME: &str = "authenticated";
}

impl FederationDirective for AuthenticatedDirective {
    fn directive_name() -> &'static str {
        Self::NAME
    }

    fn parse(_directive: &Directive<'_, String>) -> Self
    where
        Self: Sized,
    {
        Self {}
    }
}

impl Ord for AuthenticatedDirective {
    fn cmp(&self, _other: &Self) -> std::cmp::Ordering {
        std::cmp::Ordering::Equal
    }
}

impl PartialOrd for AuthenticatedDirective {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct RequiresScopesDirective {
    pub scopes: AstValue,
}

impl RequiresScopesDirective {
    pub const NAME: &str = "requiresScopes";
}

impl FederationDirective for RequiresScopesDirective {
    fn directive_name() -> &'static str {
        Self::NAME
    }

    fn parse(directive: &Directive<'_, String>) -> Self
    where
        Self: Sized,
    {
        let mut result = Self {
            scopes: AstValue::Null,
        };

        for (arg_name, arg_value) in &directive.arguments {
            if arg_name.eq("scopes") {
                // I don't like the fact it's converted from graphql_parser::Value
                // to our Value from the ast module.
                // It mixes ASTs from different modules,
                // but it's the only way to avoid using lifetimes here,
                // or refactoring big chunk of code to support `Result<T,E>` here.
                // We pass `scopes` as `Value`, so that the higher level code can
                // validate it and handle it correctly with `Result<T,E>`.
                // The rule here is that we only "read",
                // so other modules can verify and validate it.
                result.scopes = arg_value.into()
            }
        }

        result
    }
}

impl Ord for RequiresScopesDirective {
    fn cmp(&self, _other: &Self) -> std::cmp::Ordering {
        std::cmp::Ordering::Equal
    }
}

impl PartialOrd for RequiresScopesDirective {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
