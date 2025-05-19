use std::collections::BTreeMap;

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

impl From<&ParserValue<'_, String>> for Value {
    fn from(value: &ParserValue<'_, String>) -> Self {
        match value {
            ParserValue::Variable(name) => Value::Variable(name.to_owned()),
            ParserValue::Int(i) => Value::Int(i.as_i64().unwrap()),
            ParserValue::Float(f) => Value::Float(f.to_owned()),
            ParserValue::String(s) => Value::String(s.to_owned()),
            ParserValue::Boolean(b) => Value::Boolean(b.to_owned()),
            ParserValue::Null => Value::Null,
            ParserValue::Enum(e) => Value::Enum(e.to_owned()),
            ParserValue::List(l) => Value::List(l.into_iter().map(Value::from).collect()),
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
