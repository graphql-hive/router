use std::{collections::HashSet, fmt, time::Duration};

use schemars::{json_schema, JsonSchema};
use serde::{
    de::{self, Visitor},
    Deserialize, Deserializer, Serialize, Serializer,
};

use crate::primitives::value_or_expression::ValueOrExpression;

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct CoprocessorConfig {
    /// Endpoint for the external coprocessor service.
    ///
    /// Supported formats:
    /// - `http://host[:port][/path]`
    /// - `unix:///absolute/path/to/socket.sock`
    /// - `unix:///absolute/path/to/socket.sock?path=/request/path`
    pub url: CoprocessorEndpoint,

    /// Transport protocol used to call the coprocessor service.
    pub protocol: CoprocessorProtocol,

    #[serde(
        default = "default_coprocessor_timeout",
        deserialize_with = "humantime_serde::deserialize",
        serialize_with = "humantime_serde::serialize"
    )]
    #[schemars(with = "String")]
    /// Per-stage timeout for a coprocessor call.
    ///
    /// Defaults to `1s`.
    pub timeout: Duration,

    #[serde(default)]
    /// Stage-specific configuration.
    pub stages: CoprocessorStagesConfig,
}

fn default_coprocessor_timeout() -> Duration {
    Duration::from_secs(1)
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CoprocessorProtocol {
    /// HTTP/1.1 over TCP.
    Http1,
    /// HTTP/2 over TLS (currently unsupported and rejected).
    Http2,
    /// HTTP/2 cleartext over TCP.
    H2c,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct CoprocessorStagesConfig {
    #[serde(default)]
    /// Hooks around the router HTTP boundary
    pub router: CoprocessorRouterStageConfig,
    #[serde(default)]
    /// Hooks around GraphQL processing
    pub graphql: CoprocessorGraphqlStageConfig,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct CoprocessorRouterStageConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Configuration for `router.request` hook.
    pub request: Option<CoprocessorHookConfig<CoprocessorRouterRequestIncludeConfig>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Configuration for `router.response` hook.
    pub response: Option<CoprocessorHookConfig<CoprocessorRouterResponseIncludeConfig>>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct CoprocessorGraphqlStageConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Configuration for `graphql.request` hook.
    pub request: Option<CoprocessorHookConfig<CoprocessorGraphqlRequestIncludeConfig>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Configuration for `graphql.analysis` hook.
    pub analysis: Option<CoprocessorHookConfig<CoprocessorGraphqlAnalysisIncludeConfig>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Configuration for `graphql.response` hook.
    pub response: Option<CoprocessorHookConfig<CoprocessorGraphqlResponseIncludeConfig>>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct CoprocessorHookConfig<I: Default> {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Optional condition expression.
    ///
    /// The hook runs only when this expression evaluates to `true`.
    pub condition: Option<ValueOrExpression<bool>>,
    #[serde(default)]
    /// Selects which fields are included in the coprocessor payload for this hook.
    pub include: I,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct CoprocessorRouterRequestIncludeConfig {
    #[serde(default)]
    /// Include the inbound HTTP request body.
    pub body: bool,
    #[serde(default)]
    /// Include request context.
    ///
    /// Values:
    /// - `false`: no context
    /// - `true`: full context
    /// - list: selected context keys
    pub context: ContextSelection,
    #[serde(default)]
    /// Include inbound HTTP request headers.
    pub headers: bool,
    #[serde(default)]
    /// Include inbound HTTP request method.
    pub method: bool,
    #[serde(default)]
    /// Include inbound HTTP request path.
    pub path: bool,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct CoprocessorRouterResponseIncludeConfig {
    #[serde(default)]
    /// Include outbound HTTP response body.
    pub body: bool,
    #[serde(default)]
    /// Include request context.
    ///
    /// Values:
    /// - `false`: no context
    /// - `true`: full context
    /// - list: selected context keys
    pub context: ContextSelection,
    #[serde(default)]
    /// Include outbound HTTP response headers.
    pub headers: bool,
    #[serde(default)]
    /// Include outbound HTTP response status code.
    pub status_code: bool,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct CoprocessorGraphqlRequestIncludeConfig {
    #[serde(default)]
    /// Include GraphQL request body fields.
    ///
    /// Accepts `true`, `false`, or a list of fields.
    pub body: GraphqlBodySelection,
    #[serde(default)]
    /// Include request context.
    ///
    /// Values:
    /// - `false`: no context
    /// - `true`: full context
    /// - list: selected context keys
    pub context: ContextSelection,
    #[serde(default)]
    /// Include request headers.
    pub headers: bool,
    #[serde(default)]
    /// Include request method.
    pub method: bool,
    #[serde(default)]
    /// Include request path.
    pub path: bool,
    #[serde(default)]
    /// Include the current public schema SDL.
    pub sdl: bool,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct CoprocessorGraphqlResponseIncludeConfig {
    #[serde(default)]
    /// Include GraphQL response body.
    pub body: bool,
    #[serde(default)]
    /// Include request context.
    ///
    /// Values:
    /// - `false`: no context
    /// - `true`: full context
    /// - list: selected context keys
    pub context: ContextSelection,
    #[serde(default)]
    /// Include response headers.
    pub headers: bool,
    #[serde(default)]
    /// Include response status code.
    pub status_code: bool,
    #[serde(default)]
    /// Include the current public schema SDL.
    pub sdl: bool,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct CoprocessorGraphqlAnalysisIncludeConfig {
    #[serde(default)]
    /// Include GraphQL request body fields.
    ///
    /// Accepts `true`, `false`, or a list of fields.
    pub body: GraphqlBodySelection,
    #[serde(default)]
    /// Include request context.
    ///
    /// Values:
    /// - `false`: no context
    /// - `true`: full context
    /// - list: selected context keys
    pub context: ContextSelection,
    #[serde(default)]
    /// Include request headers.
    pub headers: bool,
    #[serde(default)]
    /// Include request method.
    pub method: bool,
    #[serde(default)]
    /// Include request path.
    pub path: bool,
    #[serde(default)]
    /// Include the current public schema SDL.
    pub sdl: bool,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GraphqlBodyField {
    /// Include the GraphQL query string.
    Query,
    /// Include the GraphQL operation name.
    OperationName,
    /// Include GraphQL variables.
    Variables,
    /// Include GraphQL extensions.
    Extensions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
/// Selection set for GraphQL body fields included in coprocessor payloads.
///
/// Serialized forms:
/// - `true` => all body fields
/// - `false` => no body fields
/// - list => selected body fields
pub struct GraphqlBodySelection {
    /// Include `query`.
    pub query: bool,
    /// Include `operationName`.
    pub operation_name: bool,
    /// Include `variables`.
    pub variables: bool,
    /// Include `extensions`.
    pub extensions: bool,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(untagged)]
enum GraphqlBodySelectionRepr {
    Bool(bool),
    List(Vec<GraphqlBodyField>),
}

impl GraphqlBodySelection {
    pub const fn all() -> Self {
        Self {
            query: true,
            operation_name: true,
            variables: true,
            extensions: true,
        }
    }

    pub const fn none() -> Self {
        Self {
            query: false,
            operation_name: false,
            variables: false,
            extensions: false,
        }
    }

    pub const fn is_empty(&self) -> bool {
        !self.query && !self.operation_name && !self.variables && !self.extensions
    }

    pub const fn is_all(&self) -> bool {
        self.query && self.operation_name && self.variables && self.extensions
    }
}

impl Serialize for GraphqlBodySelection {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let repr = if self.is_all() {
            GraphqlBodySelectionRepr::Bool(true)
        } else if self.is_empty() {
            GraphqlBodySelectionRepr::Bool(false)
        } else {
            let mut fields = Vec::with_capacity(4);
            if self.query {
                fields.push(GraphqlBodyField::Query);
            }
            if self.operation_name {
                fields.push(GraphqlBodyField::OperationName);
            }
            if self.variables {
                fields.push(GraphqlBodyField::Variables);
            }
            if self.extensions {
                fields.push(GraphqlBodyField::Extensions);
            }

            GraphqlBodySelectionRepr::List(fields)
        };

        repr.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for GraphqlBodySelection {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let repr = GraphqlBodySelectionRepr::deserialize(deserializer)?;

        Ok(match repr {
            GraphqlBodySelectionRepr::Bool(true) => GraphqlBodySelection::all(),
            GraphqlBodySelectionRepr::Bool(false) => GraphqlBodySelection::none(),
            GraphqlBodySelectionRepr::List(fields) => {
                let mut selection = GraphqlBodySelection::none();
                for field in fields {
                    match field {
                        GraphqlBodyField::Query => selection.query = true,
                        GraphqlBodyField::OperationName => selection.operation_name = true,
                        GraphqlBodyField::Variables => selection.variables = true,
                        GraphqlBodyField::Extensions => selection.extensions = true,
                    }
                }
                selection
            }
        })
    }
}

impl JsonSchema for GraphqlBodySelection {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "GraphqlBodySelection".into()
    }

    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        <GraphqlBodySelectionRepr as JsonSchema>::json_schema(generator)
    }

    fn inline_schema() -> bool {
        true
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
/// Selection of request-context entries included in coprocessor payloads.
///
/// Serialized forms:
/// - `true` => include full context
/// - `false` => include no context
/// - list => include only selected context keys
pub struct ContextSelection {
    all: bool,
    keys: HashSet<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(untagged)]
enum ContextSelectionRepr {
    Bool(bool),
    List(Vec<String>),
}

impl ContextSelection {
    pub fn all() -> Self {
        Self {
            all: true,
            keys: HashSet::new(),
        }
    }

    pub fn none() -> Self {
        Self {
            all: false,
            keys: HashSet::new(),
        }
    }

    pub fn list(keys: Vec<String>) -> Self {
        Self {
            all: false,
            keys: HashSet::from_iter(keys),
        }
    }

    pub const fn is_all(&self) -> bool {
        self.all
    }

    pub fn is_none(&self) -> bool {
        !self.all && self.keys.is_empty()
    }

    pub fn is_some(&self) -> bool {
        !self.is_none()
    }

    pub fn keys(&self) -> impl ExactSizeIterator<Item = &str> + '_ {
        self.keys.iter().map(|k| k.as_str())
    }
}

impl Serialize for ContextSelection {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let repr = if self.is_all() {
            ContextSelectionRepr::Bool(true)
        } else if self.is_none() {
            ContextSelectionRepr::Bool(false)
        } else {
            ContextSelectionRepr::List(Vec::from_iter(self.keys.iter().cloned()))
        };

        repr.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ContextSelection {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let repr = ContextSelectionRepr::deserialize(deserializer)?;

        Ok(match repr {
            ContextSelectionRepr::Bool(true) => ContextSelection::all(),
            ContextSelectionRepr::Bool(false) => ContextSelection::none(),
            ContextSelectionRepr::List(keys) => ContextSelection::list(keys),
        })
    }
}

impl JsonSchema for ContextSelection {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "ContextSelection".into()
    }

    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        <ContextSelectionRepr as JsonSchema>::json_schema(generator)
    }

    fn inline_schema() -> bool {
        true
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct CoprocessorExecutionRequestIncludeConfig {
    #[serde(default)]
    /// Include execution request body.
    pub body: bool,
    #[serde(default)]
    /// Include request context.
    ///
    /// Values:
    /// - `false`: no context
    /// - `true`: full context
    /// - list: selected context keys
    pub context: ContextSelection,
    #[serde(default)]
    /// Include request headers.
    pub headers: bool,
    #[serde(default)]
    /// Include request method.
    pub method: bool,
    #[serde(default)]
    /// Include request path.
    pub path: bool,
    #[serde(default)]
    /// Include the current public schema SDL.
    pub sdl: bool,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct CoprocessorExecutionResponseIncludeConfig {
    #[serde(default)]
    /// Include execution response body.
    pub body: bool,
    #[serde(default)]
    /// Include request context.
    ///
    /// Values:
    /// - `false`: no context
    /// - `true`: full context
    /// - list: selected context keys
    pub context: ContextSelection,
    #[serde(default)]
    /// Include response headers.
    pub headers: bool,
    #[serde(default)]
    /// Include response status code.
    pub status_code: bool,
    #[serde(default)]
    /// Include the current public schema SDL.
    pub sdl: bool,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct CoprocessorSubgraphRequestIncludeConfig {
    #[serde(default)]
    /// Include outbound subgraph request body.
    pub body: bool,
    #[serde(default)]
    /// Include request context.
    ///
    /// Values:
    /// - `false`: no context
    /// - `true`: full context
    /// - list: selected context keys
    pub context: ContextSelection,
    #[serde(default)]
    /// Include outbound subgraph request headers.
    pub headers: bool,
    #[serde(default)]
    /// Include outbound subgraph request method.
    pub method: bool,
    #[serde(default)]
    /// Include outbound subgraph request URI.
    pub uri: bool,
    #[serde(default)]
    /// Include the current public schema SDL.
    pub sdl: bool,
    #[serde(default)]
    /// Include target subgraph service name.
    pub service_name: bool,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct CoprocessorSubgraphResponseIncludeConfig {
    #[serde(default)]
    /// Include subgraph response body.
    pub body: bool,
    #[serde(default)]
    /// Include request context.
    ///
    /// Values:
    /// - `false`: no context
    /// - `true`: full context
    /// - list: selected context keys
    pub context: ContextSelection,
    #[serde(default)]
    /// Include subgraph response headers.
    pub headers: bool,
    #[serde(default)]
    /// Include subgraph response status code.
    pub status_code: bool,
    #[serde(default)]
    /// Include the current public schema SDL.
    pub sdl: bool,
    #[serde(default)]
    /// Include target subgraph service name.
    pub service_name: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Target endpoint for coprocessor communication.
pub enum CoprocessorEndpoint {
    Http {
        /// HTTP endpoint URL in `http://host[:port][/path]` form.
        url: String,
    },
    Unix {
        /// Absolute path to Unix domain socket file.
        socket_path: String,
        /// Request path to use when talking over Unix socket.
        request_path: String,
    },
}

impl CoprocessorEndpoint {
    fn parse(value: &str) -> Result<Self, String> {
        if value.starts_with("https://") {
            return Err("coprocessor.url with https scheme is not supported yet".to_string());
        }

        if value.starts_with("http://") {
            let parsed = value
                .parse::<http::Uri>()
                .map_err(|error| format!("invalid http URL: {error}"))?;

            if parsed.scheme_str() != Some("http") {
                return Err("coprocessor.url must use http scheme".to_string());
            }

            if parsed.authority().is_none() {
                return Err("coprocessor.url must include host (and optional port)".to_string());
            }

            return Ok(Self::Http {
                url: value.to_string(),
            });
        }

        if let Some(rest) = value.strip_prefix("unix://") {
            if !rest.starts_with('/') {
                return Err("unix coprocessor.url must include an absolute socket path".to_string());
            }

            let (socket_path, query) = match rest.split_once('?') {
                Some((socket_path, query)) => (socket_path, Some(query)),
                None => (rest, None),
            };

            if socket_path.len() <= 1 {
                return Err("unix coprocessor.url socket path cannot be empty".to_string());
            }

            let mut request_path = "/".to_string();

            if let Some(query) = query {
                if query.is_empty() {
                    return Err("unix coprocessor.url query cannot be empty".to_string());
                }

                let mut path_found = false;

                for pair in query.split('&') {
                    if pair.is_empty() {
                        continue;
                    }

                    let (key, value) = pair.split_once('=').ok_or_else(|| {
                        "unix coprocessor.url query parameters must use key=value format"
                            .to_string()
                    })?;

                    if key != "path" {
                        return Err(format!(
                            "unsupported unix coprocessor.url query parameter '{key}'"
                        ));
                    }

                    if path_found {
                        return Err(
                            "unix coprocessor.url query parameter 'path' can be provided only once"
                                .to_string(),
                        );
                    }

                    request_path = value.to_string();
                    path_found = true;
                }

                if !request_path.starts_with('/') {
                    return Err(
                        "unix coprocessor.url query parameter 'path' must start with '/'"
                            .to_string(),
                    );
                }

                if request_path.is_empty() {
                    return Err(
                        "unix coprocessor.url query parameter 'path' cannot be empty".to_string(),
                    );
                }
            }

            return Ok(Self::Unix {
                socket_path: socket_path.to_string(),
                request_path,
            });
        }

        Err("coprocessor.url must use one of the supported schemes: http:// or unix://".to_string())
    }

    fn to_config_string(&self) -> String {
        match self {
            CoprocessorEndpoint::Http { url } => url.clone(),
            CoprocessorEndpoint::Unix {
                socket_path,
                request_path,
            } => {
                if request_path == "/" {
                    format!("unix://{socket_path}")
                } else {
                    format!("unix://{socket_path}?path={request_path}")
                }
            }
        }
    }
}

impl Serialize for CoprocessorEndpoint {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_config_string())
    }
}

impl JsonSchema for CoprocessorEndpoint {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "CoprocessorEndpoint".into()
    }

    fn json_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        json_schema!({
            "type": "string",
            "description": "Coprocessor endpoint URL. Supported forms: http://host[:port][/path], unix:///path/to/socket.sock, unix:///path/to/socket.sock?path=/api/v1"
        })
    }

    fn inline_schema() -> bool {
        true
    }
}

struct CoprocessorEndpointVisitor;

impl<'de> Visitor<'de> for CoprocessorEndpointVisitor {
    type Value = CoprocessorEndpoint;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str(
            "a coprocessor endpoint URL (http://... or unix:///... with optional ?path=/...)",
        )
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        CoprocessorEndpoint::parse(value).map_err(E::custom)
    }

    fn visit_borrowed_str<E>(self, value: &'de str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        self.visit_str(value)
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        self.visit_str(&value)
    }
}

impl<'de> Deserialize<'de> for CoprocessorEndpoint {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_str(CoprocessorEndpointVisitor)
    }
}
