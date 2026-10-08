use graphql_tools::parser::schema::{Directive, Value};

use super::directives::FederationDirective;

#[derive(Debug, Default, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ContextDirective {
    pub name: String,
}

impl ContextDirective {
    pub const NAME: &str = "context";
}

impl FederationDirective for ContextDirective {
    fn directive_name() -> &'static str {
        Self::NAME
    }

    fn parse(directive: &Directive<'_, String>) -> Self {
        let mut result = Self::default();

        for (arg_name, arg_value) in &directive.arguments {
            if arg_name == "name" {
                if let Value::String(value) = arg_value {
                    result.name = value.clone();
                }
            }
        }

        result
    }
}
