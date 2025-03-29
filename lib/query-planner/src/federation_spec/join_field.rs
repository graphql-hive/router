use graphql_parser_hive_fork::schema::{Directive, Value};

#[derive(Debug, Default, Clone, PartialEq, Eq, Hash)]
pub struct JoinFieldDirective {
    pub graph_id: Option<String>,
    pub requires: Option<String>,
    pub provides: Option<String>,
    pub type_in_graph: Option<String>,
    pub external: Option<bool>,
    pub override_value: Option<String>,
    pub used_overridden: Option<bool>,
}

impl JoinFieldDirective {
    pub const NAME: &str = "join__field";

    pub fn is(directive: &Directive<'_, String>) -> bool {
        directive.name == Self::NAME
    }
}

impl From<&Directive<'_, String>> for JoinFieldDirective {
    fn from(directive: &Directive<'_, String>) -> Self {
        let mut result = Self::default();

        for (arg_name, arg_value) in &directive.arguments {
            if arg_name.eq("graph") {
                match arg_value {
                    Value::String(value) => result.graph_id = Some(value.clone()),
                    Value::Enum(value) => result.graph_id = Some(value.clone()),
                    _ => {}
                }
            } else if arg_name.eq("requires") {
                if let Value::String(value) = arg_value {
                    result.requires = Some(value.clone())
                }
            } else if arg_name.eq("provides") {
                if let Value::String(value) = arg_value {
                    result.provides = Some(value.clone())
                }
            } else if arg_name.eq("type") {
                if let Value::String(value) = arg_value {
                    result.type_in_graph = Some(value.clone())
                }
            } else if arg_name.eq("external") {
                if let Value::Boolean(value) = arg_value {
                    result.external = Some(*value)
                }
            } else if arg_name.eq("override") {
                if let Value::String(value) = arg_value {
                    result.override_value = Some(value.clone())
                }
            } else if arg_name.eq("usedOverridden") {
                if let Value::Boolean(value) = arg_value {
                    result.used_overridden = Some(*value)
                }
            }
        }

        result
    }
}
