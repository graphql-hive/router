use serde::{de, Deserialize, Deserializer, Serialize};
use sonic_rs::Value;
use std::fmt;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphQLError {
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locations: Option<Vec<GraphQLErrorLocation>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<Vec<GraphQLErrorPathSegment>>,
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

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct GraphQLErrorLocation {
    pub line: usize,
    pub column: usize,
}

#[derive(Clone, Debug, Serialize)]
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
