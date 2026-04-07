use std::fmt;

use schemars::{json_schema, JsonSchema};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbsolutePath(String);

impl AbsolutePath {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AbsolutePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl TryFrom<String> for AbsolutePath {
    type Error = String;

    fn try_from(path: String) -> Result<Self, Self::Error> {
        if !path.starts_with('/') {
            return Err(format!(
                "path must be absolute (start with /), got: {path:?}"
            ));
        }
        Ok(AbsolutePath(path))
    }
}

impl TryFrom<&str> for AbsolutePath {
    type Error = String;

    fn try_from(path: &str) -> Result<Self, Self::Error> {
        AbsolutePath::try_from(path.to_string())
    }
}

impl Serialize for AbsolutePath {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl JsonSchema for AbsolutePath {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "AbsolutePath".into()
    }

    fn json_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        json_schema!({
            "type": "string",
            "description": "An absolute path starting with /.",
            "pattern": "^/"
        })
    }

    fn inline_schema() -> bool {
        true
    }
}

struct AbsolutePathVisitor;

impl<'de> serde::de::Visitor<'de> for AbsolutePathVisitor {
    type Value = AbsolutePath;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("an absolute path starting with / (e.g., \"/callback\")")
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        AbsolutePath::try_from(value).map_err(serde::de::Error::custom)
    }

    fn visit_borrowed_str<E>(self, value: &'de str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        self.visit_str(value)
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        self.visit_str(&value)
    }
}

impl<'de> Deserialize<'de> for AbsolutePath {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_str(AbsolutePathVisitor)
    }
}
