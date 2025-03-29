use graphql_parser_hive_fork::schema::{Directive, Value};

#[derive(Debug, Default, Clone)]
pub struct JoinGraphDirective {
    pub name: String,
    pub url: String,
}

impl JoinGraphDirective {
    pub const NAME: &str = "join__graph";

    pub fn is(directive: &Directive<'_, String>) -> bool {
        directive.name == Self::NAME
    }
}

impl From<&Directive<'_, String>> for JoinGraphDirective {
    fn from(directive: &Directive<'_, String>) -> Self {
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
