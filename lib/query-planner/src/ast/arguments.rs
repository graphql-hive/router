use std::{collections::BTreeMap, fmt::Display};

use serde::{Deserialize, Serialize};

use super::value::Value;
use graphql_parser::query::Value as ParserValue;

#[derive(Clone, Debug, Deserialize, Serialize, Default)]
pub struct ArgumentsMap {
    arguments_map: BTreeMap<String, Value>,
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
        let mut arguments_map = BTreeMap::new();
        for (key, value) in args {
            arguments_map.insert(key, value);
        }
        Self { arguments_map }
    }
}

impl From<&Vec<(String, ParserValue<'_, String>)>> for ArgumentsMap {
    fn from(args: &Vec<(String, ParserValue<'_, String>)>) -> Self {
        let mut arguments_map = BTreeMap::new();
        for (key, value) in args {
            let value = Value::from(value);
            arguments_map.insert(key.to_string(), value);
        }
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
}

impl Display for ArgumentsMap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if !self.arguments_map.is_empty() {
            let args: Vec<String> = self
                .arguments_map
                .iter()
                .map(|(k, v)| format!("{}: {}", k, v))
                .collect();

            return write!(f, "{}", args.join(", "));
        }

        Ok(())
    }
}
