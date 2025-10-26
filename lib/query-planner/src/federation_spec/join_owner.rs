use graphql_parser::schema::{Directive, Value};

use super::directives::FederationDirective;

#[derive(Debug, Clone, Eq, PartialEq, Default)]
pub struct JoinOwnerDirective {
    pub graph_id: String,
}

impl JoinOwnerDirective {
    pub const NAME: &str = "join__owner";
}

impl FederationDirective for JoinOwnerDirective {
    fn directive_name() -> &'static str {
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
                    Value::String(value) => result.graph_id = value.clone(),
                    Value::Enum(value) => result.graph_id = value.clone(),
                    _ => {}
                }
            }
        }

        result
    }
}

impl Ord for JoinOwnerDirective {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.graph_id.cmp(&other.graph_id)
    }
}

impl PartialOrd for JoinOwnerDirective {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
