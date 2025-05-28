use std::collections::HashMap;

pub fn value_from_ast(
    value: &graphql_parser::query::Value<'static, String>,
    variables: &Option<HashMap<String, serde_json::Value>>,
) -> serde_json::Value {
    match value {
        graphql_parser::query::Value::Null => serde_json::Value::Null,
        graphql_parser::query::Value::Boolean(b) => serde_json::Value::Bool(*b),
        graphql_parser::query::Value::String(s) => serde_json::Value::String(s.to_string()),
        graphql_parser::query::Value::Enum(e) => serde_json::Value::String(e.to_string()),
        // TODO: Handle variable parsing errors here just like in GraphQL-JS
        graphql_parser::query::Value::Int(n) => serde_json::Value::Number(
            serde_json::Number::from(n.as_i64().expect("Failed to coerce")),
        ),
        graphql_parser::query::Value::Float(n) => {
            serde_json::Value::Number(serde_json::Number::from_f64(*n).expect("Failed to coerce"))
        }
        graphql_parser::query::Value::List(l) => {
            serde_json::Value::Array(l.iter().map(|v| value_from_ast(v, variables)).collect())
        }
        graphql_parser::query::Value::Object(o) => serde_json::Value::Object(
            o.iter()
                .map(|(k, v)| (k.to_string(), value_from_ast(v, variables)))
                .collect(),
        ),
        graphql_parser::query::Value::Variable(var_name) => {
            if let Some(variables_map) = variables {
                if let Some(value) = variables_map.get(var_name) {
                    value.clone() // Return the value from the variables map
                } else {
                    serde_json::Value::Null // If variable not found, return null
                }
            } else {
                serde_json::Value::Null // If no variables provided, return null
            }
        }
    }
}
