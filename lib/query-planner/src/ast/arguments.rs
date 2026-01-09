use std::{
    collections::BTreeMap,
    fmt::Display,
    hash::{DefaultHasher, Hash, Hasher},
};

use serde::{Deserialize, Serialize};

use super::value::Value;
use graphql_tools::parser::query::{Text as ParserText, Value as ParserValue};

#[derive(Clone, Debug, Deserialize, Serialize, Default)]
pub struct ArgumentsMap {
    #[serde(flatten)]
    arguments_map: BTreeMap<String, Value>,
}

impl Hash for ArgumentsMap {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.arguments_map.hash(state);
    }
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

impl<'a, T: ParserText<'a>> From<Vec<(T::Value, ParserValue<'a, T>)>> for ArgumentsMap
where
    T::Value: AsRef<str>,
{
    fn from(args: Vec<(T::Value, ParserValue<'a, T>)>) -> Self {
        let arguments_map = args
            .into_iter()
            .map(|(key, value)| (key.as_ref().to_string(), (&value).into()))
            .collect();
        Self { arguments_map }
    }
}

impl ArgumentsMap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn hash_u64(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.hash(&mut hasher);
        hasher.finish()
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

    pub fn keys(&self) -> impl Iterator<Item = &String> {
        self.arguments_map.keys()
    }
}

impl Display for ArgumentsMap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut iter = self.arguments_map.iter().peekable();
        while let Some((k, v)) = iter.next() {
            write!(f, "{}: {}", k, v)?;
            if iter.peek().is_some() {
                write!(f, ", ")?;
            }
        }
        Ok(())
    }
}
