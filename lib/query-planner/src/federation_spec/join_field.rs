use graphql_parser::schema::{Directive, Value};

use super::directives::FederationDirective;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct JoinFieldDirective {
    pub graph_id: Option<String>,
    pub requires: Option<String>,
    pub provides: Option<String>,
    pub type_in_graph: Option<String>,
    pub external: bool,
    pub override_value: Option<String>,
    pub used_overridden: bool,
}

// Kamil: I added allow(clippy), because I prefer to define the defaults explicitly,
// instead of relying on Default macro. It's not obvious that a default for `bool` is `false`.
#[allow(clippy::derivable_impls)]
impl Default for JoinFieldDirective {
    fn default() -> Self {
        Self {
            graph_id: Default::default(),
            requires: None,
            provides: None,
            type_in_graph: None,
            external: false,
            override_value: None,
            used_overridden: false,
        }
    }
}

impl JoinFieldDirective {
    pub const NAME: &str = "join__field";
}

impl FederationDirective for JoinFieldDirective {
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
                    result.external = *value
                }
            } else if arg_name.eq("override") {
                if let Value::String(value) = arg_value {
                    result.override_value = Some(value.clone())
                }
            } else if arg_name.eq("usedOverridden") {
                if let Value::Boolean(value) = arg_value {
                    result.used_overridden = *value
                }
            }
        }

        result
    }
}

impl Ord for JoinFieldDirective {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.graph_id.cmp(&other.graph_id)
    }
}

impl PartialOrd for JoinFieldDirective {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
