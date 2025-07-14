use hashbrown::HashMap;

use serde_json::Map;

pub fn value_from_ast(
    value: &graphql_parser::query::Value<'static, String>,
    variables: &Option<HashMap<String, serde_json::Value>>,
) -> Result<serde_json::Value, String> {
    match value {
        graphql_parser::query::Value::Null => Ok(serde_json::Value::Null),
        graphql_parser::query::Value::Boolean(b) => Ok(serde_json::Value::Bool(*b)),
        graphql_parser::query::Value::String(s) => Ok(serde_json::Value::String(s.to_string())),
        graphql_parser::query::Value::Enum(e) => Ok(serde_json::Value::String(e.to_string())),
        // TODO: Handle variable parsing errors here just like in GraphQL-JS
        graphql_parser::query::Value::Int(n) => {
            let n = n.as_i64().ok_or_else(|| "Failed to coerce".to_string())?;
            let n = serde_json::Number::from(n);
            Ok(serde_json::Value::Number(n))
        }
        graphql_parser::query::Value::Float(n) => {
            let n = serde_json::Number::from_f64(*n);
            n.map_or_else(
                || Err("Failed to coerce".to_string()),
                |num| Ok(serde_json::Value::Number(num)),
            )
        }
        graphql_parser::query::Value::List(l) => {
            let list: Result<Vec<serde_json::Value>, String> =
                l.iter().map(|v| value_from_ast(v, variables)).collect();

            match list {
                Err(e) => Err(e),
                Ok(vec) => Ok(serde_json::Value::Array(vec)),
            }
        }
        graphql_parser::query::Value::Object(o) => {
            let obj: Result<Map<String, serde_json::Value>, String> = o
                .iter()
                .map(|(k, v)| value_from_ast(v, variables).map(|val| (k.to_string(), val)))
                .collect();

            match obj {
                Err(e) => Err(e),
                Ok(map) => {
                    // Convert BTreeMap<String, serde_json::Value> to serde_json::Value::Object
                    Ok(serde_json::Value::Object(map))
                }
            }
        }
        graphql_parser::query::Value::Variable(var_name) => {
            if let Some(variables_map) = variables {
                if let Some(value) = variables_map.get(var_name) {
                    Ok(value.clone()) // Return the value from the variables map
                } else {
                    Ok(serde_json::Value::Null) // If variable not found, return null
                }
            } else {
                Ok(serde_json::Value::Null) // If no variables provided, return null
            }
        }
    }
}
