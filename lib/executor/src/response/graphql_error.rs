use graphql_parser::Pos;
use graphql_tools::validation::utils::ValidationError;
use serde::{de, Deserialize, Deserializer, Serialize};
use sonic_rs::Value;
use std::{collections::HashMap, fmt};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphQLError {
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locations: Option<Vec<GraphQLErrorLocation>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<GraphQLErrorPath>,
    pub extensions: Option<Value>,
}

impl From<String> for GraphQLError {
    fn from(message: String) -> Self {
        GraphQLError {
            message,
            locations: None,
            path: None,
            extensions: None,
        }
    }
}

impl From<&ValidationError> for GraphQLError {
    fn from(val: &ValidationError) -> Self {
        GraphQLError {
            message: val.message.to_string(),
            locations: Some(val.locations.iter().map(|pos| pos.into()).collect()),
            path: None,
            extensions: None,
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

#[derive(Clone, Debug, Deserialize, Serialize, Default)]
pub struct GraphQLErrorPath {
    #[serde(flatten)]
    pub segments: Vec<GraphQLErrorPathSegment>,
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
