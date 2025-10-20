use graphql_parser::Pos;
use graphql_tools::validation::utils::ValidationError;
use serde::{de, ser::SerializeSeq, Deserialize, Deserializer, Serialize};
use sonic_rs::Value;
use std::{collections::HashMap, fmt};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphQLError {
    pub message: String,
    #[serde(default, skip_serializing_if = "is_none_or_empty")]
    pub locations: Option<Vec<GraphQLErrorLocation>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<GraphQLErrorPath>,
    #[serde(default, skip_serializing_if = "GraphQLErrorExtensions::is_empty")]
    pub extensions: GraphQLErrorExtensions,
}

fn is_none_or_empty<T>(opt: &Option<Vec<T>>) -> bool {
    opt.as_ref().is_none_or(|v| v.is_empty())
}

impl From<String> for GraphQLError {
    fn from(message: String) -> Self {
        GraphQLError {
            message,
            locations: None,
            path: None,
            extensions: GraphQLErrorExtensions::default(),
        }
    }
}

impl From<&str> for GraphQLError {
    fn from(message: &str) -> Self {
        GraphQLError {
            message: message.to_string(),
            locations: None,
            path: None,
            extensions: GraphQLErrorExtensions::default(),
        }
    }
}

impl From<&ValidationError> for GraphQLError {
    fn from(val: &ValidationError) -> Self {
        GraphQLError {
            message: val.message.to_string(),
            locations: Some(val.locations.iter().map(|pos| pos.into()).collect()),
            path: None,
            extensions: GraphQLErrorExtensions::new_from_code("GRAPHQL_VALIDATION_FAILED"),
        }
    }
}

impl From<&Pos> for GraphQLErrorLocation {
    fn from(val: &Pos) -> Self {
        GraphQLErrorLocation {
            line: val.line,
            column: val.column,
        }
    }
}

impl GraphQLError {
    pub fn entity_index_and_path<'a>(&'a self) -> Option<EntityIndexAndPath<'a>> {
        self.path.as_ref().and_then(|p| p.entity_index_and_path())
    }

    pub fn normalize_entity_error(
        self,
        entity_index_error_map: &HashMap<&usize, Vec<GraphQLErrorPath>>,
    ) -> Vec<GraphQLError> {
        if let Some(entity_index_and_path) = &self.entity_index_and_path() {
            if let Some(entity_error_paths) =
                entity_index_error_map.get(&entity_index_and_path.entity_index)
            {
                return entity_error_paths
                    .iter()
                    .map(|error_path| {
                        let mut new_error_path = error_path.clone();
                        new_error_path.extend_from_slice(entity_index_and_path.rest_of_path);
                        GraphQLError {
                            path: Some(new_error_path),
                            ..self.clone()
                        }
                    })
                    .collect();
            }
        }
        vec![self]
    }

    pub fn from_message_and_extensions(
        message: String,
        extensions: GraphQLErrorExtensions,
    ) -> Self {
        GraphQLError {
            message,
            locations: None,
            path: None,
            extensions,
        }
    }

    pub fn add_subgraph_name(mut self, subgraph_name: &str) -> Self {
        self.extensions
            .service_name
            .get_or_insert(subgraph_name.to_string());
        self.extensions
            .code
            .get_or_insert("DOWNSTREAM_SERVICE_ERROR".to_string());
        self
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct GraphQLErrorLocation {
    pub line: usize,
    pub column: usize,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub enum GraphQLErrorPathSegment {
    String(String),
    Index(usize),
}

impl<'de> Deserialize<'de> for GraphQLErrorPathSegment {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct PathSegmentVisitor;

        impl<'de> de::Visitor<'de> for PathSegmentVisitor {
            type Value = GraphQLErrorPathSegment;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a string or an integer for a GraphQL path segment")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(GraphQLErrorPathSegment::String(value.to_owned()))
            }

            fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(GraphQLErrorPathSegment::String(value))
            }

            fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(GraphQLErrorPathSegment::Index(value as usize))
            }

            fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                if value < 0 {
                    return Err(E::custom(format!(
                        "path segment must be a non-negative integer, but got {}",
                        value
                    )));
                }
                Ok(GraphQLErrorPathSegment::Index(value as usize))
            }
        }

        deserializer.deserialize_any(PathSegmentVisitor)
    }
}

#[derive(Clone, Debug, Default)]
pub struct GraphQLErrorPath {
    pub segments: Vec<GraphQLErrorPathSegment>,
}

impl<'de> Serialize for GraphQLErrorPath {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut seq = serializer.serialize_seq(Some(self.segments.len()))?;
        for segment in &self.segments {
            match segment {
                GraphQLErrorPathSegment::String(s) => seq.serialize_element(s)?,
                GraphQLErrorPathSegment::Index(i) => seq.serialize_element(i)?,
            }
        }
        seq.end()
    }
}

impl<'de> Deserialize<'de> for GraphQLErrorPath {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct PathVisitor;

        impl<'de> de::Visitor<'de> for PathVisitor {
            type Value = GraphQLErrorPath;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a list of strings and integers for a GraphQL error path")
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: de::SeqAccess<'de>,
            {
                let mut segments = Vec::new();
                while let Some(segment) = seq.next_element::<GraphQLErrorPathSegment>()? {
                    segments.push(segment);
                }
                Ok(GraphQLErrorPath { segments })
            }
        }

        deserializer.deserialize_seq(PathVisitor)
    }
}

pub struct EntityIndexAndPath<'a> {
    pub entity_index: usize,
    pub rest_of_path: &'a [GraphQLErrorPathSegment],
}

impl GraphQLErrorPath {
    pub fn with_capacity(capacity: usize) -> Self {
        GraphQLErrorPath {
            segments: Vec::with_capacity(capacity),
        }
    }
    pub fn concat(&self, segment: GraphQLErrorPathSegment) -> Self {
        let mut new_path = self.segments.clone();
        new_path.push(segment);
        GraphQLErrorPath { segments: new_path }
    }

    pub fn concat_index(&self, index: usize) -> Self {
        self.concat(GraphQLErrorPathSegment::Index(index))
    }

    pub fn concat_str(&self, field: String) -> Self {
        self.concat(GraphQLErrorPathSegment::String(field))
    }

    pub fn extend_from_slice(&mut self, other: &[GraphQLErrorPathSegment]) {
        self.segments.extend_from_slice(other);
    }

    pub fn entity_index_and_path<'a>(&'a self) -> Option<EntityIndexAndPath<'a>> {
        match &self.segments.as_slice() {
            [GraphQLErrorPathSegment::String(maybe_entities), GraphQLErrorPathSegment::Index(entity_index), rest_of_path @ ..]
                if maybe_entities == "_entities" =>
            {
                Some(EntityIndexAndPath {
                    entity_index: *entity_index,
                    rest_of_path,
                })
            }
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GraphQLErrorExtensions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_name: Option<String>,
    #[serde(flatten)]
    pub extensions: HashMap<String, Value>,
}

impl GraphQLErrorExtensions {
    pub fn new_from_code(code: &str) -> Self {
        GraphQLErrorExtensions {
            code: Some(code.to_string()),
            service_name: None,
            extensions: HashMap::new(),
        }
    }
    pub fn new_from_code_and_service_name(code: &str, service_name: &str) -> Self {
        GraphQLErrorExtensions {
            code: Some(code.to_string()),
            service_name: Some(service_name.to_string()),
            extensions: HashMap::new(),
        }
    }
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.extensions.get(key)
    }

    pub fn set(&mut self, key: String, value: Value) {
        self.extensions.insert(key, value);
    }

    pub fn is_empty(&self) -> bool {
        self.code.is_none() && self.service_name.is_none() && self.extensions.is_empty()
    }
}
