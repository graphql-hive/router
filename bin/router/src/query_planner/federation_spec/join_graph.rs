use graphql_tools::parser::schema::{Directive, Value};

use super::directives::FederationDirective;

#[derive(Debug, Default, Clone, Eq, PartialEq)]
pub struct JoinGraphDirective {
    pub name: String,
    pub url: String,
}

impl JoinGraphDirective {
    pub const NAME: &str = "join__graph";
}

impl FederationDirective for JoinGraphDirective {
    fn directive_name() -> &'static str {
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

impl Ord for JoinGraphDirective {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.name.cmp(&other.name)
    }
}

impl PartialOrd for JoinGraphDirective {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
