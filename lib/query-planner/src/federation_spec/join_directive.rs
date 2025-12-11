use graphql_parser::schema::{Directive, Value};

use super::directives::FederationDirective;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct JoinDirectiveDirective {
    pub graphs: Vec<String>,
    pub name: String,
}

impl JoinDirectiveDirective {
    pub const NAME: &str = "join__directive";
}

impl FederationDirective for JoinDirectiveDirective {
    fn directive_name() -> &'static str {
        Self::NAME
    }

    fn parse(directive: &Directive<'_, String>) -> Self
    where
        Self: Sized,
    {
        let mut result = Self::default();

        for (arg_name, arg_value) in &directive.arguments {
            match (arg_name.as_str(), arg_value) {
                ("graphs", Value::List(values)) => {
                    for value in values {
                        if let Value::String(graph_name) = value {
                            result.graphs.push(graph_name.to_owned());
                        }
                    }
                }
                ("name", Value::String(name)) => {
                    result.name = name.to_owned();
                }
                _ => {}
            }
        }

        result
    }
}

impl Ord for JoinDirectiveDirective {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.graphs
            .cmp(&other.graphs)
            .then(self.name.cmp(&other.name))
    }
}

impl PartialOrd for JoinDirectiveDirective {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
