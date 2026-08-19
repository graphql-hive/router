use graphql_tools::parser::schema::{Directive, Value};

use super::directives::FederationDirective;

#[derive(Debug, Default, Clone, Eq, PartialEq)]
pub struct JoinUnionMemberDirective {
    pub graph: String,
    pub member: String,
}

impl JoinUnionMemberDirective {
    pub const NAME: &str = "join__unionMember";
}

impl FederationDirective for JoinUnionMemberDirective {
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
                    Value::String(value) => result.graph = value.clone(),
                    Value::Enum(value) => result.graph = value.clone(),
                    _ => {}
                }
            } else if arg_name.eq("member") {
                if let Value::String(value) = arg_value {
                    result.member = value.clone()
                }
            }
        }

        result
    }
}

impl Ord for JoinUnionMemberDirective {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.graph.cmp(&other.graph)
    }
}

impl PartialOrd for JoinUnionMemberDirective {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
