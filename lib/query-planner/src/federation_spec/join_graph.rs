use graphql_parser::schema::{Directive, Value};

use super::directives::FederationDirective;

#[derive(Debug, Default, Clone)]
pub struct JoinGraphDirective {
    pub name: String,
    pub url: String,
}

impl JoinGraphDirective {
    pub const NAME: &str = "join__graph";
}

impl<'a> FederationDirective<'a> for JoinGraphDirective {
    fn directive_name() -> &'a str {
        Self::NAME
    }

    fn parse(directive: &Directive<'_, String>) -> Self
    where
        Self: Sized,
    {
        let mut result = Self::default();

        for (arg_name, arg_value) in &directive.arguments {
            if arg_name.eq("name") {
                if let Value::String(value) = arg_value {
                    result.name = value.clone()
                }
            } else if arg_name.eq("url") {
                if let Value::String(value) = arg_value {
                    result.url = value.clone()
                }
            }
        }

        result
    }
}
