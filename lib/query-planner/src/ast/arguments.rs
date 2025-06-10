use std::{collections::BTreeMap, fmt::Display};

use serde::{Deserialize, Serialize};

use super::value::Value;
use graphql_parser::query::Value as ParserValue;

#[derive(Clone, Debug, Deserialize, Serialize, Default)]
pub struct ArgumentsMap {
    #[serde(flatten)]
    arguments_map: BTreeMap<String, Value>,
}

impl<'a> IntoIterator for &'a ArgumentsMap {
    type Item = (&'a String, &'a Value);
    type IntoIter = std::collections::btree_map::Iter<'a, String, Value>;

    fn into_iter(self) -> Self::IntoIter {
        self.arguments_map.iter()
    }
}

impl PartialEq for ArgumentsMap {
    fn eq(&self, other: &Self) -> bool {
        self.arguments_map == other.arguments_map
    }
}

impl From<(String, Value)> for ArgumentsMap {
    fn from((key, value): (String, Value)) -> Self {
        let mut arguments_map = BTreeMap::new();
        arguments_map.insert(key, value);
        Self { arguments_map }
    }
}

impl From<Vec<(String, Value)>> for ArgumentsMap {
    fn from(args: Vec<(String, Value)>) -> Self {
        Self {
            arguments_map: args.into_iter().collect(),
        }
    }
}

impl From<&Vec<(String, ParserValue<'_, String>)>> for ArgumentsMap {
    fn from(args: &Vec<(String, ParserValue<'_, String>)>) -> Self {
        let arguments_map = args
            .iter()
            .map(|(key, value)| (key.clone(), Value::from(value)))
            .collect();
        Self { arguments_map }
    }
}

impl From<Vec<(String, ParserValue<'_, String>)>> for ArgumentsMap {
    fn from(value: Vec<(String, ParserValue<'_, String>)>) -> Self {
        let arguments_map = value
            .iter()
            .map(|(key, value)| (key.clone(), Value::from(value)))
            .collect();
        Self { arguments_map }
    }
}

impl From<&mut Vec<(String, ParserValue<'_, String>)>> for ArgumentsMap {
    fn from(args: &mut Vec<(String, ParserValue<'_, String>)>) -> Self {
        let arguments_map = args
            .iter()
            .map(|(key, value)| (key.clone(), Value::from(value)))
            .collect();
        Self { arguments_map }
    }
}

impl ArgumentsMap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_argument(&mut self, key: String, value: Value) {
        self.arguments_map.insert(key, value);
    }

    pub fn has_argument(&self, key: &str) -> bool {
        self.arguments_map.contains_key(key)
    }

    pub fn get_argument(&self, key: &str) -> Option<&Value> {
        self.arguments_map.get(key)
    }

    pub fn is_empty(&self) -> bool {
        self.arguments_map.is_empty()
    }

    pub fn values(&self) -> impl Iterator<Item = &Value> {
        self.arguments_map.values()
    }
}

impl Display for ArgumentsMap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.arguments_map.is_empty() {
            return Ok(());
        }

        let args: Vec<String> = self
            .arguments_map
            .iter()
            .map(|(k, v)| format!("{}: {}", k, v))
            .collect();

        write!(f, "{}", args.join(", "))
    }
}
