use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::config::primitives::http_header::HttpHeaderName;

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct LaboratoryConfig {
    /// Enables/disables the Hive Laboratory interface. By default, the Hive Laboratory interface is enabled.
    ///
    /// You can override this setting by setting the `LABORATORY_ENABLED` environment variable to `true` or `false`.
    #[serde(default = "default_laboratory_enabled")]
    pub enabled: bool,
    /// Headers sent on every request the Laboratory makes to the router, as a map of header name to
    /// value.
    ///
    /// Unlike an operation's `headers`, these are not shown or editable in the Laboratory UI: they
    /// are attached to the underlying request transport. A header set on an individual operation
    /// overrides a global header of the same name.
    ///
    /// > These are embedded in the HTML page served to every browser that opens the Laboratory and
    /// > are visible via "view source". Do not put secrets here.
    ///
    /// ```yaml
    /// laboratory:
    ///   global_headers:
    ///     X-Env: staging
    /// ```
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub global_headers: BTreeMap<HttpHeaderName, String>,
    /// Operations to pre-populate the Laboratory with.
    ///
    /// Each operation opens in its own tab the first time a browser sees it. Operations the user
    /// creates themselves are preserved, and if the user closes a seeded tab it stays closed. The
    /// content of a seeded operation is refreshed from this configuration on every page load, so
    /// edits a user makes to a seeded operation are not kept.
    ///
    /// > Seeded operations are embedded in the HTML page served to every browser that opens the
    /// > Laboratory and are visible via "view source". Do not put secrets in `headers`.
    ///
    /// ```yaml
    /// laboratory:
    ///   operations:
    ///     - name: GetHello
    ///       query: |
    ///         query GetHello {
    ///           hello
    ///         }
    ///       variables:
    ///         limit: 10
    ///       headers:
    ///         X-Env: staging
    /// ```
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub operations: Vec<LaboratoryOperationConfig>,
    /// Collections to pre-populate the Laboratory with.
    ///
    /// A collection is a named, reusable group of operations shown in the Laboratory's sidebar. Use
    /// this to hand users a labelled set of standard queries they can browse and run. Each
    /// collection must contain at least one operation.
    ///
    /// Seeded collections are refreshed from this configuration on every page load: a user can edit
    /// one during a session, but it resets on reload. To change a seeded collection permanently,
    /// change it here in the router configuration. Collections a user creates themselves are never
    /// touched.
    ///
    /// > Seeded collections are embedded in the HTML page served to every browser that opens the
    /// > Laboratory and are visible via "view source". Do not put secrets in `headers`.
    ///
    /// ```yaml
    /// laboratory:
    ///   collections:
    ///     - name: Onboarding
    ///       operations:
    ///         - name: GetHello
    ///           query: |
    ///             query GetHello {
    ///               hello
    ///             }
    ///           headers:
    ///             X-Env: staging
    /// ```
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub collections: Vec<LaboratoryCollectionConfig>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct LaboratoryOperationConfig {
    /// The name of the operation. Used as the tab title, and must be unique across all seeded
    /// operations.
    pub name: String,
    /// The GraphQL document of the operation.
    pub query: String,
    /// The operation's variables, as a JSON object (map of variable name to value). Values may be
    /// nested objects, arrays, numbers, booleans or strings.
    ///
    /// Values support `{{name}}` references to the Laboratory's environment variables; a templated
    /// value resolves to a string.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variables: Option<BTreeMap<String, serde_json::Value>>,
    /// Headers to send with this operation, as a map of header name to value. These apply only to
    /// this operation.
    ///
    /// Values support `{{name}}` references to the Laboratory's environment variables.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headers: Option<BTreeMap<HttpHeaderName, String>>,
    /// The operation's GraphQL extensions, as a JSON object.
    ///
    /// Values support `{{name}}` references to the Laboratory's environment variables; a templated
    /// value resolves to a string.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extensions: Option<BTreeMap<String, serde_json::Value>>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct LaboratoryCollectionConfig {
    /// The name of the collection. Used as the sidebar label, and must be unique across all seeded
    /// collections.
    pub name: String,
    /// The operations in this collection. Operation names must be unique within the collection.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub operations: Vec<LaboratoryOperationConfig>,
}

fn default_laboratory_enabled() -> bool {
    true
}

impl Default for LaboratoryConfig {
    fn default() -> Self {
        Self {
            enabled: default_laboratory_enabled(),
            global_headers: BTreeMap::new(),
            operations: Vec::new(),
            collections: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use config_rs::{Config, ConfigError, File, FileFormat};

    use super::*;

    fn parse(yaml: &str) -> Result<LaboratoryConfig, ConfigError> {
        Config::builder()
            .add_source(File::from_str(yaml, FileFormat::Yaml))
            .build()?
            .try_deserialize::<LaboratoryConfig>()
    }

    #[test]
    fn defaults_when_only_enabled_is_set() {
        let config = parse("enabled: true").expect("should parse");

        assert!(config.enabled);
        assert!(config.global_headers.is_empty());
        assert!(config.operations.is_empty());
        assert!(config.collections.is_empty());
    }

    #[test]
    fn parses_global_headers() {
        let config = parse(
            r#"
global_headers:
  X-Env: staging
  X-Team: payments
"#,
        )
        .expect("should parse");

        assert_eq!(
            config.global_headers.get(&HttpHeaderName::from("X-Env")),
            Some(&"staging".to_string())
        );
        assert_eq!(
            config.global_headers.get(&HttpHeaderName::from("X-Team")),
            Some(&"payments".to_string())
        );
    }

    #[test]
    fn parses_operations() {
        let config = parse(
            r#"
operations:
  - name: GetHello
    query: "query GetHello { hello }"
    headers:
      X-Env: staging
"#,
        )
        .expect("should parse");

        assert_eq!(config.operations.len(), 1);
        let operation = &config.operations[0];
        assert_eq!(operation.name, "GetHello");
        assert_eq!(operation.query, "query GetHello { hello }");
        assert_eq!(
            operation
                .headers
                .as_ref()
                .and_then(|h| h.get(&HttpHeaderName::from("X-Env"))),
            Some(&"staging".to_string())
        );
        assert!(operation.variables.is_none());
        assert!(operation.extensions.is_none());
    }

    #[test]
    fn parses_nested_variables_as_a_native_object() {
        let config = parse(
            r#"
operations:
  - name: Search
    query: "query Search { search }"
    variables:
      filter:
        status: active
        tags: [premium, trial]
      limit: 10
"#,
        )
        .expect("should parse");

        let variables = config.operations[0]
            .variables
            .as_ref()
            .expect("variables should be present");

        // Types and nesting survive the YAML -> JSON value round-trip.
        assert_eq!(variables["limit"], 10);
        assert_eq!(variables["filter"]["status"], "active");
        assert_eq!(variables["filter"]["tags"][0], "premium");
    }

    #[test]
    fn rejects_non_object_variables_at_config_load() {
        // Typing variables as a map means a non-object is rejected by config parsing itself,
        // located to the field, with no custom validation needed.
        let error = parse(
            r#"
operations:
  - name: Search
    query: "query Search { search }"
    variables: [1, 2, 3]
"#,
        )
        .expect_err("a non-object variables should be rejected by config parsing")
        .to_string();

        assert!(
            error.contains("expected a map"),
            "unexpected error: {error}"
        );
        assert!(error.contains("variables"), "unexpected error: {error}");
    }

    #[test]
    fn parses_collections() {
        let config = parse(
            r#"
collections:
  - name: Onboarding
    operations:
      - name: GetHello
        query: "query GetHello { hello }"
        headers:
          X-Env: staging
      - name: ListUsers
        query: "query ListUsers { users { id } }"
  - name: Admin
    operations:
      - name: PurgeCache
        query: "mutation PurgeCache { purge }"
"#,
        )
        .expect("should parse");

        assert_eq!(config.collections.len(), 2);

        let onboarding = &config.collections[0];
        assert_eq!(onboarding.name, "Onboarding");
        assert_eq!(onboarding.operations.len(), 2);
        assert_eq!(onboarding.operations[0].name, "GetHello");
        assert_eq!(
            onboarding.operations[0]
                .headers
                .as_ref()
                .and_then(|h| h.get(&HttpHeaderName::from("X-Env"))),
            Some(&"staging".to_string())
        );

        assert_eq!(config.collections[1].name, "Admin");
        assert_eq!(config.collections[1].operations.len(), 1);
    }

    #[test]
    fn parses_a_config_with_only_collections() {
        let config = parse(
            r#"
collections:
  - name: Onboarding
    operations:
      - name: GetHello
        query: "query GetHello { hello }"
"#,
        )
        .expect("should parse");

        assert!(config.operations.is_empty());
        assert_eq!(config.collections.len(), 1);
    }

    #[test]
    fn rejects_unknown_fields() {
        let error = parse(
            r#"
default_headers:
  X-Env: staging
"#,
        )
        .expect_err("unknown fields should be rejected");

        assert!(error.to_string().contains("default_headers"));
    }

    #[test]
    fn rejects_unknown_fields_inside_a_collection() {
        let error = parse(
            r#"
collections:
  - name: Onboarding
    description: not a real field
    operations: []
"#,
        )
        .expect_err("unknown collection fields should be rejected");

        assert!(error.to_string().contains("description"));
    }
}
