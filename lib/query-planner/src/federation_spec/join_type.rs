use graphql_parser::schema::{Directive, Value};

use super::directives::FederationDirective;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct JoinTypeDirective {
    pub graph_id: String,
    pub key: Option<String>,
    pub extension: bool,
    pub resolvable: bool,
    pub is_interface_object: bool,
}

impl Default for JoinTypeDirective {
    fn default() -> Self {
        Self {
            graph_id: Default::default(),
            key: None,
            extension: false,
            resolvable: true,
            is_interface_object: false,
        }
    }
}

impl JoinTypeDirective {
    pub const NAME: &str = "join__type";
}

impl<'a> FederationDirective<'a> for JoinTypeDirective {
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
                    result.extension = *value
                }
            } else if arg_name.eq("resolvable") {
                if let Value::Boolean(value) = arg_value {
                    result.resolvable = *value
                }
            } else if arg_name.eq("is_interface_object") {
                if let Value::Boolean(value) = arg_value {
                    result.is_interface_object = *value
                }
            }
        }

        result
    }
}

impl Ord for JoinTypeDirective {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.graph_id.cmp(&other.graph_id)
    }
}

impl PartialOrd for JoinTypeDirective {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
