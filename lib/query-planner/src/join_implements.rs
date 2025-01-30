use graphql_parser_hive_fork::query::Directive;

#[derive(Debug, Default, Clone)]
pub struct JoinImplementsDirective {
    pub graph: String,
    pub interface: String,
}

impl JoinImplementsDirective {
    pub const NAME: &str = "join__implements";

    pub fn is(directive: &Directive<'_, String>) -> bool {
        directive.name == Self::NAME
    }
}

impl From<&Directive<'_, String>> for JoinImplementsDirective {
    fn from(directive: &Directive<'_, String>) -> Self {
        let mut result = Self::default();

        for (arg_name, arg_value) in &directive.arguments {
            if arg_name.eq("graph") {
                match arg_value {
                    graphql_parser_hive_fork::schema::Value::String(value) => {
                        result.graph = value.clone()
                    }
                    graphql_parser_hive_fork::schema::Value::Enum(value) => {
                        result.graph = value.clone()
                    }
                    _ => {}
                }
            } else if arg_name.eq("interface") {
                if let graphql_parser_hive_fork::schema::Value::String(value) = arg_value {
                    result.interface = value.clone()
                }
            }
        }

        result
    }
}
