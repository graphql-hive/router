use graphql_parser_hive_fork::schema::{Directive, Value};

#[derive(Debug, Default, PartialEq, Eq)]
pub struct JoinEnumValueDirective {
    pub graph: String,
}

impl JoinEnumValueDirective {
    pub const NAME: &str = "join__enumValue";

    pub fn is(directive: &Directive<'_, String>) -> bool {
        directive.name == Self::NAME
    }
}

impl From<&Directive<'_, String>> for JoinEnumValueDirective {
    fn from(directive: &Directive<'_, String>) -> Self {
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
