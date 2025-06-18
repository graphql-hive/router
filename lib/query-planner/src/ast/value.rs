use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Display,
    hash::Hash,
    mem,
};

use graphql_parser::query::Value as ParserValue;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub enum Value {
    Variable(String),
    Int(i64),
    Float(f64),
    String(String),
    Boolean(bool),
    Null,
    Enum(String),
    List(Vec<Value>),
    Object(BTreeMap<String, Value>),
}

impl Hash for Value {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        match self {
            Value::Variable(name) => name.hash(state),
            Value::Int(i) => i.hash(state),
            Value::Float(f) => f.to_bits().hash(state),
            Value::String(s) => s.hash(state),
            Value::Boolean(b) => b.hash(state),
            Value::Null => 0_u8.hash(state),
            Value::Enum(e) => e.hash(state),
            Value::List(l) => l.hash(state),
            Value::Object(o) => o.hash(state),
        }
    }
}

impl Value {
    pub fn variable_usages(&self) -> BTreeSet<String> {
        match self {
            Value::Variable(name) => BTreeSet::from([name.clone()]),
            Value::List(values) => values.iter().flat_map(Value::variable_usages).collect(),
            Value::Object(map) => map.values().flat_map(Value::variable_usages).collect(),
            _ => BTreeSet::new(),
        }
    }
}

impl From<&ParserValue<'_, String>> for Value {
    fn from(value: &ParserValue<'_, String>) -> Self {
        match value {
            ParserValue::Variable(name) => Value::Variable(name.to_owned()),
            // TODO: Consider `TryFrom` and handle this in a better way
            ParserValue::Int(i) => {
                Value::Int(i.as_i64().expect("GraphQL integer value out of i64 range"))
            }
            ParserValue::Float(f) => Value::Float(*f),
            ParserValue::String(s) => Value::String(s.to_owned()),
            ParserValue::Boolean(b) => Value::Boolean(*b),
            ParserValue::Null => Value::Null,
            ParserValue::Enum(e) => Value::Enum(e.to_owned()),
            ParserValue::List(l) => Value::List(l.iter().map(Value::from).collect()),
            ParserValue::Object(o) => {
                let mut map = BTreeMap::new();
                for (k, v) in o {
                    map.insert(k.to_string(), Value::from(v));
                }
                Value::Object(map)
            }
        }
    }
}

impl From<&mut ParserValue<'_, String>> for Value {
    fn from(value: &mut ParserValue<'_, String>) -> Self {
        match value {
            ParserValue::Variable(name) => Value::Variable(mem::take(name)),
            // TODO: Consider `TryFrom` and handle this in a better way
            ParserValue::Int(i) => {
                Value::Int(i.as_i64().expect("GraphQL integer value out of i64 range"))
            }
            ParserValue::Float(f) => Value::Float(mem::take(f)),
            ParserValue::String(s) => Value::String(mem::take(s)),
            ParserValue::Boolean(b) => Value::Boolean(mem::take(b)),
            ParserValue::Null => Value::Null,
            ParserValue::Enum(e) => Value::Enum(mem::take(e)),
            ParserValue::List(l) => Value::List(l.iter_mut().map(Value::from).collect()),
            ParserValue::Object(o) => {
                let mut map = BTreeMap::new();
                for (k, v) in o {
                    map.insert(k.to_string(), Value::from(v));
                }
                Value::Object(map)
            }
        }
    }
}

impl From<&Value> for sonic_rs::Value {
    fn from(value: &Value) -> Self {
        match value {
            Value::Null => sonic_rs::Value::new_null(),
            Value::Int(n) => (*n).into(),
            Value::Boolean(b) => (*b).into(),
            Value::Enum(s) => s.into(),
            Value::Float(n) => match sonic_rs::Value::new_f64(*n) {
                Some(num) => num,
                None => sonic_rs::Value::new_null(),
            },
            Value::List(l) => {
                let mut array_value = sonic_rs::Value::new_array_with(l.len());

                for val in l.iter() {
                    array_value.append_value(val.into());
                }

                array_value
            }
            Value::Object(o) => {
                let mut object_value = sonic_rs::Value::new_object_with(o.len());

                for (k, v) in o.iter() {
                    object_value.insert(k, v.into());
                }

                object_value
            }
            Value::String(s) => s.into(),
            Value::Variable(_var_name) => sonic_rs::Value::new_null(),
        }
    }
}

// impl From<&mut Value> for sonic_rs::Value {
//     fn from(value: &mut Value) -> Self {
//         match value {
//             Value::Null => sonic_rs::Value::new_null(),
//             Value::Int(n) => (*n).into(),
//             Value::Boolean(b) => (*b).into(),
//             Value::Enum(s) => s.into(),
//             Value::Float(n) => match sonic_rs::Value::new_f64(*n) {
//                 Some(num) => num,
//                 None => sonic_rs::Value::new_null(),
//             },
//             Value::List(l) => {
//                 let mut array_value = sonic_rs::Value::new_array_with(l.len());

//                 for val in l.iter() {
//                     array_value.append_value(val.into());
//                 }

//                 array_value
//             }
//             Value::Object(o) => {
//                 let mut object_value = sonic_rs::Value::new_object_with(o.len());

//                 for (k, v) in o.iter() {
//                     object_value.insert(k, v.into());
//                 }

//                 object_value
//             }
//             Value::String(s) => s.into(),
//             Value::Variable(_var_name) => sonic_rs::Value::new_null(),
//         }
//     }
// }

impl Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Variable(name) => write!(f, "${}", name),
            Value::Int(i) => write!(f, "{}", i),
            Value::Float(fl) => write!(f, "{}", fl),
            Value::String(s) => write!(f, "\"{}\"", s),
            Value::Boolean(b) => write!(f, "{}", b),
            Value::Null => write!(f, "null"),
            Value::Enum(e) => write!(f, "{}", e),
            Value::List(l) => {
                let values: Vec<String> = l.iter().map(|v| v.to_string()).collect();
                write!(f, "[{}]", values.join(", "))
            }
            Value::Object(o) => {
                let entries: Vec<String> = o.iter().map(|(k, v)| format!("{}: {}", k, v)).collect();
                write!(f, "{{{}}}", entries.join(", "))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    #[test]
    fn test_value_display() {
        use super::Value;

        insta::assert_snapshot!(
          Value::Int(42),
          @r#"42"#);

        insta::assert_snapshot!(
          Value::Float(42.2),
          @r#"42.2"#);

        insta::assert_snapshot!(
          Value::String("test".to_string()),
          @r#""test""#);

        insta::assert_snapshot!(
          Value::Boolean(false),
          @r#"false"#);

        insta::assert_snapshot!(
          Value::Enum("SOME".to_string()),
          @r#"SOME"#);

        insta::assert_snapshot!(
          Value::Variable("test".to_string()),
          @r#"$test"#);

        insta::assert_snapshot!(
          Value::Object(BTreeMap::from([
            ("key1".to_string(), Value::Int(42)),
            ("key2".to_string(), Value::String("value".to_string())),
          ])),
          @r#"{key1: 42, key2: "value"}"#);

        insta::assert_snapshot!(
          Value::List(vec![Value::Int(42), Value::Int(10)]),
          @"[42, 10]");
    }
}
