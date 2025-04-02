use graphql_parser_hive_fork::schema::{Directive, Value};

#[derive(Debug, Default, Clone)]
pub struct JoinUnionMemberDirective {
    graph: String,
    member: String,
}

impl JoinUnionMemberDirective {
    pub const NAME: &str = "join__unionMember";

    pub fn is(directive: &Directive<'_, String>) -> bool {
        directive.name == Self::NAME
    }
}

impl From<&Directive<'_, String>> for JoinUnionMemberDirective {
    fn from(directive: &Directive<'_, String>) -> Self {
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
