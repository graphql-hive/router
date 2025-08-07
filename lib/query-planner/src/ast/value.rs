use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Display,
    hash::Hash,
    mem,
};

use graphql_parser::query::{Text as ParserText, Value as ParserValue};
use serde::{Deserialize, Serialize};
use sonic_rs::Value as SonicValue;

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

impl<'a, T: ParserText<'a>> From<&ParserValue<'a, T>> for Value {
    fn from(value: &ParserValue<'a, T>) -> Self {
        match value {
            ParserValue::Variable(name) => Value::Variable(name.as_ref().to_string()),
            // TODO: Consider `TryFrom` and handle this in a better way
            ParserValue::Int(i) => {
                Value::Int(i.as_i64().expect("GraphQL integer value out of i64 range"))
            }
            ParserValue::Float(f) => Value::Float(*f),
            ParserValue::String(s) => Value::String(s.to_string()),
            ParserValue::Boolean(b) => Value::Boolean(*b),
            ParserValue::Null => Value::Null,
            ParserValue::Enum(e) => Value::Enum(e.as_ref().to_string()),
            ParserValue::List(l) => Value::List(l.iter().map(Value::from).collect()),
            ParserValue::Object(o) => {
                let mut map = BTreeMap::new();
                for (k, v) in o {
                    map.insert(k.as_ref().to_string(), Value::from(v));
                }
                Value::Object(map)
            }
        }
    }
}

impl<'a, T: ParserText<'a>> From<&mut ParserValue<'a, T>> for Value {
    fn from(value: &mut ParserValue<'a, T>) -> Self {
        match value {
            ParserValue::Variable(name) => Value::Variable(name.as_ref().to_owned()),
            // TODO: Consider `TryFrom` and handle this in a better way
            ParserValue::Int(i) => {
                Value::Int(i.as_i64().expect("GraphQL integer value out of i64 range"))
            }
            ParserValue::Float(f) => Value::Float(mem::take(f)),
            ParserValue::String(s) => Value::String(mem::take(s)),
            ParserValue::Boolean(b) => Value::Boolean(mem::take(b)),
            ParserValue::Null => Value::Null,
            ParserValue::Enum(e) => Value::Enum(e.as_ref().to_owned()),
            ParserValue::List(l) => Value::List(l.iter_mut().map(Value::from).collect()),
            ParserValue::Object(o) => {
                let mut map = BTreeMap::new();
                for (k, v) in o {
                    map.insert(k.as_ref().to_string(), Value::from(v));
                }
                Value::Object(map)
            }
        }
    }
}

impl From<&Value> for serde_json::Value {
    fn from(value: &Value) -> Self {
        match value {
            Value::Null => serde_json::Value::Null,
            Value::Int(n) => serde_json::Value::Number((*n).into()),
            Value::Boolean(b) => serde_json::Value::Bool(*b),
            Value::Enum(s) => serde_json::Value::String(s.to_string()),
            Value::Float(n) => {
                let number = serde_json::Number::from_f64(*n);
                match number {
                    Some(num) => serde_json::Value::Number(num),
                    None => serde_json::Value::Null, // Handle case where float conversion fails
                }
            }
            Value::List(l) => serde_json::Value::Array(l.iter().map(|v| v.into()).collect()),
            Value::Object(o) => serde_json::Value::Object(
                o.iter().map(|(k, v)| (k.to_string(), v.into())).collect(),
            ),
            Value::String(s) => serde_json::Value::String(s.to_string()),
            Value::Variable(_var_name) => serde_json::Value::Null,
        }
    }
}

impl From<&mut Value> for serde_json::Value {
    fn from(value: &mut Value) -> Self {
        match value {
            Value::Null => serde_json::Value::Null,
            Value::Int(n) => serde_json::Value::Number((mem::take(n)).into()),
            Value::Boolean(b) => serde_json::Value::Bool(mem::take(b)),
            Value::Enum(s) => serde_json::Value::String(mem::take(s)),
            Value::Float(n) => {
                let number = serde_json::Number::from_f64(mem::take(n));
                match number {
                    Some(num) => serde_json::Value::Number(num),
                    None => serde_json::Value::Null, // Handle case where float conversion fails
                }
            }
            Value::List(l) => serde_json::Value::Array(l.iter_mut().map(|v| v.into()).collect()),
            Value::Object(o) => serde_json::Value::Object(
                o.iter_mut()
                    .map(|(k, v)| (k.to_string(), v.into()))
                    .collect(),
            ),
            Value::String(s) => serde_json::Value::String(mem::take(s)),
            Value::Variable(_var_name) => serde_json::Value::Null,
        }
    }
}

impl From<&Value> for SonicValue {
    fn from(value: &Value) -> Self {
        match value {
            Value::Null => SonicValue::new_null(),
            Value::Int(n) => (*n).into(),
            Value::Boolean(b) => (*b).into(),
            Value::Enum(s) => s.into(),
            Value::Float(n) => match SonicValue::new_f64(*n) {
                Some(num) => num,
                None => SonicValue::new_null(),
            },
            Value::List(l) => {
                let mut array_value = SonicValue::new_array_with(l.len());

                for val in l.iter() {
                    array_value.append_value(val.into());
                }

                array_value
            }
            Value::Object(o) => {
                let mut object_value = SonicValue::new_object_with(o.len());

                for (k, v) in o.iter() {
                    object_value.insert(k, v.into());
                }

                object_value
            }
            Value::String(s) => s.into(),
            Value::Variable(_var_name) => SonicValue::new_null(),
        }
    }
}

impl From<&mut Value> for SonicValue {
    fn from(value: &mut Value) -> Self {
        match value {
            Value::Null => SonicValue::new_null(),
            Value::Int(n) => (mem::take(n)).into(),
            Value::Boolean(b) => mem::take(b).into(),
            Value::Enum(s) => (&mem::take(s)).into(),
            Value::Float(n) => match SonicValue::new_f64(mem::take(n)) {
                Some(num) => num,
                None => SonicValue::new_null(),
            },
            Value::List(l) => {
                let mut array_value = SonicValue::new_array_with(l.len());

                for val in l.iter_mut() {
                    array_value.append_value(val.into());
                }

                array_value
            }
            Value::Object(o) => {
                let mut object_value = SonicValue::new_object_with(o.len());

                for (k, v) in o.iter_mut() {
                    object_value.insert(k, v.into());
                }

                object_value
            }
            Value::String(s) => (&mem::take(s)).into(),
            Value::Variable(_var_name) => SonicValue::new_null(),
        }
    }
}

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
                f.write_str("[")?;
                let mut iter = l.iter().peekable();
                while let Some(v) = iter.next() {
                    write!(f, "{}", v)?;
                    if iter.peek().is_some() {
                        f.write_str(", ")?;
                    }
                }
                f.write_str("]")
            }
            Value::Object(o) => {
                f.write_str("{")?;
                let mut iter = o.iter().peekable();
                while let Some((k, v)) = iter.next() {
                    write!(f, "{}: {}", k, v)?;
                    if iter.peek().is_some() {
                        write!(f, ", ")?;
                    }
                }
                f.write_str("}")
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
