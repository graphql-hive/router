use graphql_parser::query::Value as ParserValue;
use sonic_rs::Value as SonicValue;
use std::collections::{BTreeMap, HashMap};

pub fn value_from_ast(
    value: &ParserValue<'static, String>,
    variables: &Option<HashMap<String, SonicValue>>,
) -> Result<SonicValue, String> {
    match value {
        ParserValue::Null => Ok(SonicValue::new_null()),
        ParserValue::Boolean(b) => Ok((*b).into()),
        ParserValue::String(s) => Ok(s.into()),
        ParserValue::Enum(e) => Ok(e.into()),
        // TODO: Handle variable parsing errors here just like in GraphQL-JS
        ParserValue::Int(n) => {
            let n = n.as_i64().ok_or_else(|| "Failed to coerce".to_string())?;
            Ok(n.into())
        }
        ParserValue::Float(n) => {
            let n = SonicValue::new_f64(*n);
            n.map_or_else(|| Err("Failed to coerce".to_string()), |num| Ok(num))
        }
        ParserValue::List(l) => {
            let list: Result<Vec<SonicValue>, String> =
                l.iter().map(|v| value_from_ast(v, variables)).collect();

            match list {
                Err(e) => Err(e),
                Ok(vec) => Ok(SonicValue::from_iter(vec)),
            }
        }
        ParserValue::Object(o) => {
            let obj: Result<BTreeMap<String, SonicValue>, String> = o
                .iter()
                .map(|(k, v)| value_from_ast(v, variables).map(|val| (k.to_string(), val)))
                .collect();

            match obj {
                Err(e) => Err(e),
                Ok(map) => Ok(SonicValue::from_iter(map.iter())),
            }
        }
        ParserValue::Variable(var_name) => {
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
