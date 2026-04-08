use std::{fmt, time::Duration};

use schemars::{json_schema, JsonSchema};
use serde::{
    de::{self, Visitor},
    Deserialize, Deserializer, Serialize, Serializer,
};

use crate::primitives::value_or_expression::ValueOrExpression;

#[derive(Debug, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct CoprocessorConfig {
    pub url: CoprocessorEndpoint,

    pub protocol: CoprocessorProtocol,

    #[serde(
        default = "default_coprocessor_timeout",
        deserialize_with = "humantime_serde::deserialize",
        serialize_with = "humantime_serde::serialize"
    )]
    #[schemars(with = "String")]
    pub timeout: Duration,

    #[serde(default)]
    pub stages: CoprocessorStagesConfig,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CoprocessorConfigRaw {
    url: CoprocessorEndpoint,
    protocol: CoprocessorProtocol,
    #[serde(
        default = "default_coprocessor_timeout",
        deserialize_with = "humantime_serde::deserialize"
    )]
    timeout: Duration,
    #[serde(default)]
    stages: CoprocessorStagesConfig,
}

impl<'de> Deserialize<'de> for CoprocessorConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = CoprocessorConfigRaw::deserialize(deserializer)?;

        if raw.protocol == CoprocessorProtocol::Http2 {
            return Err(de::Error::custom(
                "coprocessor.protocol=http2 is not supported yet; use protocol=http1 or protocol=h2c",
            ));
        }

        Ok(Self {
            url: raw.url,
            protocol: raw.protocol,
            timeout: raw.timeout,
            stages: raw.stages,
        })
    }
}

fn default_coprocessor_timeout() -> Duration {
    Duration::from_secs(1)
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CoprocessorProtocol {
    Http1,
    Http2,
    H2c,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct CoprocessorStagesConfig {
    #[serde(default)]
    pub router: CoprocessorRouterStageConfig,
    #[serde(default)]
    pub graphql: CoprocessorGraphqlStageConfig,
    #[serde(default)]
    pub execution: CoprocessorExecutionStageConfig,
    #[serde(default)]
    pub subgraph: CoprocessorSubgraphStageConfig,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct CoprocessorRouterStageConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<CoprocessorHookConfig<CoprocessorRouterRequestIncludeConfig>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response: Option<CoprocessorHookConfig<CoprocessorRouterResponseIncludeConfig>>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct CoprocessorGraphqlStageConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<CoprocessorHookConfig<CoprocessorGraphqlRequestIncludeConfig>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub analysis: Option<CoprocessorHookConfig<CoprocessorGraphqlAnalysisIncludeConfig>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response: Option<CoprocessorHookConfig<CoprocessorGraphqlResponseIncludeConfig>>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct CoprocessorExecutionStageConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<CoprocessorHookConfig<CoprocessorExecutionRequestIncludeConfig>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response: Option<CoprocessorHookConfig<CoprocessorExecutionResponseIncludeConfig>>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct CoprocessorSubgraphStageConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<CoprocessorHookConfig<CoprocessorSubgraphRequestIncludeConfig>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response: Option<CoprocessorHookConfig<CoprocessorSubgraphResponseIncludeConfig>>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct CoprocessorHookConfig<I: Default> {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition: Option<ValueOrExpression<bool>>,
    #[serde(default)]
    pub include: I,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct CoprocessorRouterRequestIncludeConfig {
    #[serde(default)]
    pub body: bool,
    #[serde(default)]
    pub context: bool,
    #[serde(default)]
    pub headers: bool,
    #[serde(default)]
    pub method: bool,
    #[serde(default)]
    pub path: bool,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct CoprocessorRouterResponseIncludeConfig {
    #[serde(default)]
    pub body: bool,
    #[serde(default)]
    pub context: bool,
    #[serde(default)]
    pub headers: bool,
    #[serde(default)]
    pub status_code: bool,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct CoprocessorGraphqlRequestIncludeConfig {
    #[serde(default)]
    pub body: GraphqlBodySelection,
    #[serde(default)]
    pub context: bool,
    #[serde(default)]
    pub headers: bool,
    #[serde(default)]
    pub method: bool,
    #[serde(default)]
    pub path: bool,
    #[serde(default)]
    pub sdl: bool,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct CoprocessorGraphqlResponseIncludeConfig {
    #[serde(default)]
    pub body: bool,
    #[serde(default)]
    pub context: bool,
    #[serde(default)]
    pub headers: bool,
    #[serde(default)]
    pub status_code: bool,
    #[serde(default)]
    pub sdl: bool,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct CoprocessorGraphqlAnalysisIncludeConfig {
    #[serde(default)]
    pub body: GraphqlBodySelection,
    #[serde(default)]
    pub context: bool,
    #[serde(default)]
    pub headers: bool,
    #[serde(default)]
    pub method: bool,
    #[serde(default)]
    pub path: bool,
    #[serde(default)]
    pub sdl: bool,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GraphqlBodyField {
    Query,
    OperationName,
    Variables,
    Extensions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GraphqlBodySelection {
    pub query: bool,
    pub operation_name: bool,
    pub variables: bool,
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

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct CoprocessorExecutionRequestIncludeConfig {
    #[serde(default)]
    pub body: bool,
    #[serde(default)]
    pub context: bool,
    #[serde(default)]
    pub headers: bool,
    #[serde(default)]
    pub method: bool,
    #[serde(default)]
    pub path: bool,
    #[serde(default)]
    pub sdl: bool,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct CoprocessorExecutionResponseIncludeConfig {
    #[serde(default)]
    pub body: bool,
    #[serde(default)]
    pub context: bool,
    #[serde(default)]
    pub headers: bool,
    #[serde(default)]
    pub status_code: bool,
    #[serde(default)]
    pub sdl: bool,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct CoprocessorSubgraphRequestIncludeConfig {
    #[serde(default)]
    pub body: bool,
    #[serde(default)]
    pub context: bool,
    #[serde(default)]
    pub headers: bool,
    #[serde(default)]
    pub method: bool,
    #[serde(default)]
    pub uri: bool,
    #[serde(default)]
    pub sdl: bool,
    #[serde(default)]
    pub service_name: bool,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct CoprocessorSubgraphResponseIncludeConfig {
    #[serde(default)]
    pub body: bool,
    #[serde(default)]
    pub context: bool,
    #[serde(default)]
    pub headers: bool,
    #[serde(default)]
    pub status_code: bool,
    #[serde(default)]
    pub sdl: bool,
    #[serde(default)]
    pub service_name: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoprocessorEndpoint {
    Http {
        url: String,
    },
    Unix {
        socket_path: String,
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
