use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct LaboratoryConfig {
    /// Enables/disables the Hive Laboratory interface. By default, the Hive Laboratory interface is enabled.
    ///
    /// You can override this setting by setting the `LABORATORY_ENABLED` environment variable to `true` or `false`.
    #[serde(default = "default_laboratory_enabled")]
    pub enabled: bool,
    /// A script that runs in the browser before every operation executed from the Laboratory.
    ///
    /// This is the only way to apply headers to *every* operation, including operations the user
    /// creates later. Headers set by the preflight script are the base of the request; headers set
    /// on an individual operation are merged on top and win on conflict.
    ///
    /// > **The script source is public.** It is embedded in the HTML page served to every browser
    /// > that opens the Laboratory, and is visible via "view source". Never put a secret in it.
    /// > To handle secrets, use `lab.prompt` to ask the user for them at runtime, as shown below.
    /// > Preflight is a convenience for Laboratory users, not a router authentication mechanism.
    ///
    /// ### Example: a static header for a development environment
    ///
    /// ```yaml
    /// laboratory:
    ///   preflight:
    ///     script: |
    ///       lab.request.headers.set('X-Env', 'staging');
    /// ```
    ///
    /// ### Example: prompt for a token once, then reuse it
    ///
    /// The token is entered by the user and kept in the Laboratory's environment. It never appears
    /// in the router configuration or in the served page.
    ///
    /// ```yaml
    /// laboratory:
    ///   preflight:
    ///     script: |
    ///       let token = lab.environment.get('token');
    ///       if (!token) {
    ///         token = await lab.prompt('Enter your API token', '');
    ///         lab.environment.set('token', token);
    ///       }
    ///       lab.request.headers.set('Authorization', `Bearer ${token}`);
    /// ```
    ///
    /// ### Example: sign the request with an HMAC
    ///
    /// `CryptoJS` is available in the script scope.
    ///
    /// ```yaml
    /// laboratory:
    ///   preflight:
    ///     script: |
    ///       const secret = lab.environment.get('signing_key')
    ///         ?? await lab.prompt('Enter the signing key', '');
    ///       lab.environment.set('signing_key', secret);
    ///
    ///       const timestamp = String(Date.now());
    ///       const signature = CryptoJS.HmacSHA256(timestamp, secret).toString();
    ///       lab.request.headers.set('X-Timestamp', timestamp);
    ///       lab.request.headers.set('X-Signature', signature);
    /// ```
    ///
    /// ### Available API
    ///
    /// The script runs in an isolated web worker and may use `await` at the top level.
    ///
    /// - `lab.environment.get(key)` / `.set(key, value)` / `.delete(key)`: read and write the
    ///   Laboratory's environment variables. Values persist across runs.
    /// - `lab.request.headers`: a standard `Headers` object. Whatever it contains when the script
    ///   finishes becomes the base headers of the operation.
    /// - `lab.prompt(placeholder, defaultValue)`: returns a `Promise` that resolves with the value
    ///   the user enters.
    /// - `CryptoJS`: the full crypto-js library.
    /// - `console.log` / `.warn` / `.error` / `.info`: forwarded to the Laboratory's preflight log.
    ///
    /// Environment variables can also be referenced as `{{name}}` inside an operation's `headers`,
    /// `variables` and `extensions`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preflight: Option<LaboratoryPreflightConfig>,
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
    ///       variables: '{}'
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
pub struct LaboratoryPreflightConfig {
    /// Enables/disables the preflight script. By default, a configured preflight script is enabled.
    ///
    /// You can override this setting by setting the `LABORATORY_PREFLIGHT_ENABLED` environment
    /// variable to `true` or `false`.
    #[serde(default = "default_preflight_enabled")]
    pub enabled: bool,
    /// The JavaScript source of the preflight script. An empty script is ignored.
    #[serde(default)]
    pub script: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct LaboratoryOperationConfig {
    /// The name of the operation. Used as the tab title, and must be unique across all seeded
    /// operations.
    pub name: String,
    /// The GraphQL document of the operation.
    pub query: String,
    /// The operation's variables, as a JSON object encoded in a string.
    ///
    /// Supports `{{name}}` references to the Laboratory's environment variables.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variables: Option<String>,
    /// Headers to send with this operation, as a map of header name to value.
    ///
    /// These apply only to this operation, and are merged on top of any headers set by the
    /// preflight script. To set headers on every operation, use `preflight` instead.
    ///
    /// Values support `{{name}}` references to the Laboratory's environment variables.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headers: Option<BTreeMap<String, String>>,
    /// The operation's GraphQL extensions, as a JSON object encoded in a string.
    ///
    /// Supports `{{name}}` references to the Laboratory's environment variables.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extensions: Option<String>,
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

fn default_preflight_enabled() -> bool {
    true
}

impl Default for LaboratoryConfig {
    fn default() -> Self {
        Self {
            enabled: default_laboratory_enabled(),
            preflight: None,
            operations: Vec::new(),
            collections: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use config::{Config, ConfigError, File, FileFormat};

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
        assert!(config.preflight.is_none());
        assert!(config.operations.is_empty());
        assert!(config.collections.is_empty());
    }

    #[test]
    fn parses_preflight() {
        let config = parse(
            r#"
enabled: true
preflight:
  script: |
    lab.request.headers.set('X-Env', 'staging');
"#,
        )
        .expect("should parse");

        let preflight = config.preflight.expect("preflight should be present");
        assert!(preflight.enabled, "preflight defaults to enabled");
        assert!(preflight.script.contains("X-Env"));
    }

    #[test]
    fn parses_disabled_preflight_without_a_script() {
        let config = parse(
            r#"
preflight:
  enabled: false
"#,
        )
        .expect("should parse");

        let preflight = config.preflight.expect("preflight should be present");
        assert!(!preflight.enabled);
        assert_eq!(preflight.script, "");
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
            operation.headers.as_ref().and_then(|h| h.get("X-Env")),
            Some(&"staging".to_string())
        );
        assert!(operation.variables.is_none());
        assert!(operation.extensions.is_none());
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
                .and_then(|h| h.get("X-Env")),
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

        assert!(config.preflight.is_none());
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
