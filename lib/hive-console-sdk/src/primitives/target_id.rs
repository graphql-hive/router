//! Validation for Hive Console target IDs.
//!
//! A target ID is either a slug `"$organizationSlug/$projectSlug/$targetSlug"`
//! (for example `"the-guild/graphql-hive/staging"`) or a UUID
//! (for example `"a0f4c605-6541-4350-8cfe-b31f21a4bf80"`).
//!
//! Use [`TargetId::parse`] to validate at runtime; the same regexes are
//! exposed in [`TargetId`]'s JSON Schema so misconfigs surface in YAML
//! editors before the router even starts.

use std::{borrow::Cow, fmt, str::FromStr, sync::LazyLock};

use schemars::{json_schema, JsonSchema, Schema, SchemaGenerator};
use serde::{de, Deserialize, Deserializer, Serialize, Serializer};

const SLUG_PATTERN: &str = r"^[a-zA-Z0-9-_]+\/[a-zA-Z0-9-_]+\/[a-zA-Z0-9-_]+$";
const UUID_PATTERN: &str =
    r"^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$";

static SLUG_REGEX: LazyLock<regex_automata::meta::Regex> = LazyLock::new(|| {
    regex_automata::meta::Regex::new(SLUG_PATTERN).expect("Failed to compile target_id slug regex")
});

static UUID_REGEX: LazyLock<regex_automata::meta::Regex> = LazyLock::new(|| {
    regex_automata::meta::Regex::new(UUID_PATTERN).expect("Failed to compile target_id UUID regex")
});

/// Validated Hive Console target id.
///
/// Wraps a string that has been verified to match either the slug format
/// (`$organizationSlug/$projectSlug/$targetSlug`) or the UUID format. The
/// type rejects empty / malformed values at deserialization time and
/// exposes the same regex patterns through its JSON Schema, so misconfigs
/// surface in YAML editors rather than at runtime.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TargetId(String);

#[derive(Debug, thiserror::Error)]
pub enum TargetIdParseError {
    #[error("target id cannot be empty")]
    Empty,
    #[error("invalid target id '{0}': must be either a slug '$organizationSlug/$projectSlug/$targetSlug' or a UUID 'a0f4c605-6541-4350-8cfe-b31f21a4bf80'")]
    Invalid(String),
}

impl TargetId {
    /// Parses a string into a [`TargetId`], trimming surrounding whitespace
    /// before validation. The input must match either the slug format
    /// `$organizationSlug/$projectSlug/$targetSlug` or a UUID.
    pub fn parse(value: impl Into<String>) -> Result<Self, TargetIdParseError> {
        let value = value.into();
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err(TargetIdParseError::Empty);
        }
        if !SLUG_REGEX.is_match(trimmed) && !UUID_REGEX.is_match(trimmed) {
            return Err(TargetIdParseError::Invalid(trimmed.to_string()));
        }
        Ok(Self(trimmed.to_string()))
    }

    /// Returns the underlying string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consumes the wrapper and returns the inner string.
    pub fn into_inner(self) -> String {
        self.0
    }
}

impl fmt::Display for TargetId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for TargetId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl FromStr for TargetId {
    type Err = TargetIdParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl Serialize for TargetId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for TargetId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        TargetId::parse(raw).map_err(de::Error::custom)
    }
}

impl JsonSchema for TargetId {
    fn schema_name() -> Cow<'static, str> {
        "TargetId".into()
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "A Hive Console target id, either a slug '$organizationSlug/$projectSlug/$targetSlug' (e.g. 'the-guild/graphql-hive/staging') or a UUID (e.g. 'a0f4c605-6541-4350-8cfe-b31f21a4bf80').",
            "type": "string",
            "oneOf": [
                {
                    "title": "Slug",
                    "pattern": SLUG_PATTERN
                },
                {
                    "title": "UUID",
                    "pattern": UUID_PATTERN
                }
            ]
        })
    }

    fn inline_schema() -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_slug() {
        let id = TargetId::parse("the-guild/graphql-hive/staging").expect("slug");
        assert_eq!(id.as_str(), "the-guild/graphql-hive/staging");
    }

    #[test]
    fn parses_uuid() {
        let id = TargetId::parse("a0f4c605-6541-4350-8cfe-b31f21a4bf80").expect("uuid");
        assert_eq!(id.as_str(), "a0f4c605-6541-4350-8cfe-b31f21a4bf80");
    }

    #[test]
    fn trims_whitespace() {
        let id = TargetId::parse("  the-guild/graphql-hive/staging  ").expect("slug");
        assert_eq!(id.as_str(), "the-guild/graphql-hive/staging");
    }

    #[test]
    fn rejects_empty() {
        assert!(matches!(
            TargetId::parse("   "),
            Err(TargetIdParseError::Empty)
        ));
    }

    #[test]
    fn rejects_invalid() {
        assert!(matches!(
            TargetId::parse("not a valid id"),
            Err(TargetIdParseError::Invalid(_))
        ));
        assert!(matches!(
            TargetId::parse("only/two"),
            Err(TargetIdParseError::Invalid(_))
        ));
    }

    #[test]
    fn deserializes_from_json_slug() {
        let id: TargetId =
            serde_json::from_str(r#""the-guild/graphql-hive/staging""#).expect("json slug");
        assert_eq!(id.as_str(), "the-guild/graphql-hive/staging");
    }

    #[test]
    fn deserialize_rejects_invalid() {
        let result: Result<TargetId, _> = serde_json::from_str(r#""just a string""#);
        assert!(
            result.is_err(),
            "invalid target id must fail deserialization"
        );
    }
}
