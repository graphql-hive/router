use graphql_parser::query::Directive;

use super::directives::FederationDirective;

#[derive(Debug, Default, Clone, Eq, PartialEq)]
pub struct JoinImplementsDirective {
    pub graph_id: String,
    pub interface: String,
}

impl JoinImplementsDirective {
    pub const NAME: &str = "join__implements";
}

impl<'a> FederationDirective<'a> for JoinImplementsDirective {
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
                    graphql_parser::schema::Value::String(value) => result.graph_id = value.clone(),
                    graphql_parser::schema::Value::Enum(value) => result.graph_id = value.clone(),
                    _ => {}
                }
            } else if arg_name.eq("interface") {
                if let graphql_parser::schema::Value::String(value) = arg_value {
                    result.interface = value.clone()
                }
            }
        }

        result
    }
}

impl Ord for JoinImplementsDirective {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.graph_id.cmp(&other.graph_id)
    }
}

impl PartialOrd for JoinImplementsDirective {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
