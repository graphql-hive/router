use graphql_parser::schema::{Directive, Value};

use crate::{graph::edge::OverrideLabel, state::supergraph_state::TypeNode};

use super::directives::FederationDirective;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct JoinFieldDirective {
    pub graph_id: Option<String>,
    pub requires: Option<String>,
    pub provides: Option<String>,
    pub type_in_graph: Option<TypeNode>,
    pub external: bool,
    pub override_value: Option<String>,
    pub override_label: Option<OverrideLabel>,
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
            override_label: None,
            used_overridden: false,
        }
    }
}

impl JoinFieldDirective {
    pub const NAME: &str = "join__field";

    fn parse_override_label(label: &str) -> OverrideLabel {
        if let Some(value_str) = label
            .strip_prefix("percent(")
            .and_then(|s| s.strip_suffix(')'))
        {
            let is_precision_valid = match value_str.find('.') {
                Some(dot_index) => {
                    // The decimal precision should not be longer than 8 digits
                    value_str.len() - dot_index - 1 <= 8
                }
                None => true,
            };

            if !is_precision_valid {
                panic!("Invalid precision for percentage override. Must be no more than 8 fraction digits.");
            }

            match value_str.parse::<f64>() {
                Ok(value) => {
                    if !(0.0..=100.0).contains(&value) {
                        panic!("Invalid percentage value. Must be between 0 and 100.");
                    }

                    // Multiply by 100,000,000 to scale for integer storage
                    // and to preserve 8 fraction digits.
                    // (11.12 becomes 1112000000.0)
                    // Cast to u64 for storage
                    // (11120.0 becomes 1112000000)
                    return OverrideLabel::Percentage((value * 100_000_000.0) as u64);
                }
                Err(error) => {
                    panic!(
                        "Invalid percentage value. Must be between 0 and 100. {}",
                        error
                    );
                }
            }
        }
        OverrideLabel::Custom(label.to_string())
    }
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
                    result.type_in_graph = Some(value.as_str().try_into().unwrap())
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
            } else if arg_name.eq("overrideLabel") {
                if let Value::String(value) = arg_value {
                    result.override_label = Some(Self::parse_override_label(value));
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
