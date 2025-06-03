use graphql_parser::schema::{Directive, Value};

use super::directives::FederationDirective;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct JoinEnumValueDirective {
    pub graph: String,
}

impl JoinEnumValueDirective {
    pub const NAME: &str = "join__enumValue";
}

impl<'a> FederationDirective<'a> for JoinEnumValueDirective {
    fn directive_name() -> &'a str {
        Self::NAME
    }

    fn parse(directive: &Directive<'_, String>) -> Self
    where
        Self: Sized,
    {
        let mut result = Self::default();

        for (arg_name, arg_value) in &directive.arguments {
            if arg_name.eq("graph") {
                match arg_value {
                    Value::String(value) => result.graph = value.clone(),
                    Value::Enum(value) => result.graph = value.clone(),
                    _ => {}
                }
            }
        }

        result
    }
}

impl Ord for JoinEnumValueDirective {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.graph.cmp(&other.graph)
    }
}

impl PartialOrd for JoinEnumValueDirective {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.graph.partial_cmp(&other.graph)
    }
}
