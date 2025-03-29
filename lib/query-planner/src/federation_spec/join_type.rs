use graphql_parser_hive_fork::schema::{Directive, Value};

#[derive(Debug, Default, Clone)]
pub struct JoinTypeDirective {
    pub graph_id: String,
    pub key: Option<String>,
    pub extension: Option<bool>,
    pub resolvable: Option<bool>,
    pub is_interface_object: Option<bool>,
}

impl JoinTypeDirective {
    pub const NAME: &str = "join__type";

    pub fn is(directive: &Directive<'_, String>) -> bool {
        directive.name == Self::NAME
    }
}

impl From<&Directive<'_, String>> for JoinTypeDirective {
    fn from(directive: &Directive<'_, String>) -> Self {
        let mut result = Self::default();

        for (arg_name, arg_value) in &directive.arguments {
            if arg_name.eq("graph") {
                match arg_value {
                    Value::String(value) => result.graph_id = value.clone(),
                    Value::Enum(value) => result.graph_id = value.clone(),
                    _ => {}
                }
            } else if arg_name.eq("key") {
                if let Value::String(value) = arg_value {
                    result.key = Some(value.clone())
                }
            } else if arg_name.eq("extension") {
                if let Value::Boolean(value) = arg_value {
                    result.extension = Some(*value)
                }
            } else if arg_name.eq("resolvable") {
                if let Value::Boolean(value) = arg_value {
                    result.resolvable = Some(*value)
                }
            } else if arg_name.eq("is_interface_object") {
                if let Value::Boolean(value) = arg_value {
                    result.is_interface_object = Some(*value)
                }
            }
        }

        result
    }
}
