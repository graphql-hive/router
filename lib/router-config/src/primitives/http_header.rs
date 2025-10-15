use std::{fmt, str::FromStr};

use http::HeaderName;
use schemars::{json_schema, JsonSchema};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpHeaderName(HeaderName);

impl From<HeaderName> for HttpHeaderName {
    fn from(header_name: HeaderName) -> Self {
        HttpHeaderName(header_name)
    }
}

impl HttpHeaderName {
    pub fn get_header_ref(&self) -> &HeaderName {
        &self.0
    }
}

impl From<&str> for HttpHeaderName {
    fn from(header_name: &str) -> Self {
        HttpHeaderName(HeaderName::from_str(header_name).unwrap())
    }
}

impl From<String> for HttpHeaderName {
    fn from(header_name: String) -> Self {
        HttpHeaderName(HeaderName::from_str(&header_name).unwrap())
    }
}

impl Serialize for HttpHeaderName {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.0.as_str())
    }
}

struct HeaderNameVisitor;

impl JsonSchema for HttpHeaderName {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "HttpHeaderName".into()
    }

    fn json_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        json_schema!({
            "type": "string",
            "description": "A valid HTTP header name, according to RFC 7230.",
            "pattern": "^[A-Za-z0-9!#$%&'*+\\-.^_`|~]+$"
        })
    }

    fn inline_schema() -> bool {
        true
    }
}

impl<'de> serde::de::Visitor<'de> for HeaderNameVisitor {
    type Value = HttpHeaderName;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("an HTTP header name string (e.g., \"Content-Type\")")
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        HeaderName::from_str(value)
            .map(HttpHeaderName)
            .map_err(serde::de::Error::custom)
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

impl<'de> Deserialize<'de> for HttpHeaderName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_str(HeaderNameVisitor)
    }
}
