use std::collections::{BTreeMap, HashMap};

use sonic_rs::Value as SonicValue;

pub fn value_from_ast(
    value: &graphql_parser::query::Value<'static, String>,
    variables: &Option<HashMap<String, SonicValue>>,
) -> Result<SonicValue, String> {
    match value {
        graphql_parser::query::Value::Null => Ok(SonicValue::new_null()),
        graphql_parser::query::Value::Boolean(b) => Ok((*b).into()),
        graphql_parser::query::Value::String(s) => Ok(s.into()),
        graphql_parser::query::Value::Enum(e) => Ok(e.into()),
        // TODO: Handle variable parsing errors here just like in GraphQL-JS
        graphql_parser::query::Value::Int(n) => {
            let n = n.as_i64().ok_or_else(|| "Failed to coerce".to_string())?;
            let n = SonicValue::from(n);
            Ok(n)
        }
        graphql_parser::query::Value::Float(n) => {
            let n = SonicValue::new_f64(*n);
            n.map_or_else(|| Err("Failed to coerce".to_string()), |num| Ok(num))
        }
        graphql_parser::query::Value::List(l) => {
            let list: Result<Vec<SonicValue>, String> =
                l.iter().map(|v| value_from_ast(v, variables)).collect();

            match list {
                Err(e) => Err(e),
                Ok(vec) => Ok(vec.into()),
            }
        }
        graphql_parser::query::Value::Object(o) => {
            let obj: Result<BTreeMap<String, SonicValue>, String> = o
                .iter()
                .map(|(k, v)| value_from_ast(v, variables).map(|val| (k.to_string(), val)))
                .collect();

            match obj {
                Err(e) => Err(e),
                Ok(map) => {
                    // Convert BTreeMap<String, Value> to Value::Object
                    Ok(SonicValue::from_iter(map.iter()))
                }
            }
        }
        graphql_parser::query::Value::Variable(var_name) => {
            if let Some(variables_map) = variables {
                if let Some(value) = variables_map.get(var_name) {
                    Ok(value.clone()) // Return the value from the variables map
                } else {
                    Ok(SonicValue::new_null()) // If variable not found, return null
                }
            } else {
                Ok(SonicValue::new_null()) // If no variables provided, return null
            }
        }
    }
}
