use std::collections::HashMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

type HeaderName = String;
type RegExp = String;

pub const HOP_BY_HOP_HEADERS: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    "proxy-connection",
    "host",
    "content-length",
];

pub static NEVER_JOIN_HEADERS: &[&str] = &["set-cookie", "www-authenticate"];

/// Configuration for how Router handles HTTP headers.
///
/// This allows you to define rules for which headers should be
/// propagated, removed, or set when requests are sent to subgraphs
/// or responses are sent back to clients.
#[derive(Debug, Default, Deserialize, Serialize, JsonSchema, Clone)]
pub struct HeadersConfig {
    /// Rules applied to all subgraphs (global defaults).
    #[serde(default)]
    pub all: Option<HeaderRules>,

    /// Rules applied to individual subgraphs.
    /// Keys are subgraph names as defined in the supergraph schema.
    #[serde(default)]
    pub subgraphs: Option<HashMap<String, HeaderRules>>,
}

/// Rules for a single scope (global or per subgraph).
#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, Default)]
pub struct HeaderRules {
    #[serde(default)]
    pub request: Option<Vec<RequestHeaderRule>>,

    #[serde(default)]
    pub response: Option<Vec<ResponseHeaderRule>>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(rename_all = "snake_case")]
pub enum RequestHeaderRule {
    /// Forward headers from the client request into subgraph requests.
    Propagate(RequestPropagateRule),
    /// Remove headers before sending the request to a subgraph.
    Remove(RemoveRule),
    /// Add or overwrite a header with a static or dynamic value.
    Insert(InsertRule),
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(rename_all = "snake_case")]
pub enum ResponseHeaderRule {
    /// Forward headers from subgraph responses into the final client response.
    Propagate(ResponsePropagateRule),
    /// Remove headers before sending the response to the client.
    Remove(RemoveRule),
    /// Add or overwrite a header in the response to the client.
    Insert(InsertRule),
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
pub struct RemoveRule {
    #[serde(flatten)]
    pub spec: MatchSpec,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
pub struct InsertRule {
    pub name: HeaderName,
    #[serde(flatten)]
    pub source: InsertSource,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(untagged)]
pub enum InsertSource {
    /// Static value provided in the config.
    Value { value: String },
    // Expression { expression: String },
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(untagged)]
pub enum OneOrMany<T> {
    One(T),
    Many(Vec<T>),
}

/// Match spec for header rules.
///
/// - `named`: one or more exact header names (OR semantics).
/// - `matching`: one or more regex patterns (OR semantics).
/// - `exclude`: optional list of regex patterns to subtract.
///
/// Hop-by-hop headers (connection, content-length, etc.) are **never propagated**
/// even if they match the patterns.
#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, Default)]
pub struct MatchSpec {
    /// Match headers by exact name.
    #[serde(default)]
    pub named: Option<OneOrMany<HeaderName>>,

    /// Match headers by regex pattern(s).
    #[serde(default)]
    pub matching: Option<OneOrMany<RegExp>>,

    /// Exclude headers matching these regexes, applied after `named`/`matching`.
    #[serde(default)]
    pub exclude: Option<Vec<RegExp>>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
pub struct RequestPropagateRule {
    #[serde(flatten)]
    pub spec: MatchSpec,

    /// Optionally rename the header when forwarding.
    #[serde(default)]
    pub rename: Option<HeaderName>,

    /// If the header is missing, set a default value.
    #[serde(default)]
    pub default: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum AggregationAlgo {
    /// Take the first value encountered and ignore later ones.
    First,
    /// Overwrite with the last value encountered (default behavior).
    Last,
    /// Append all values, into a comma-separated string.
    Append,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
pub struct ResponsePropagateRule {
    #[serde(flatten)]
    pub spec: MatchSpec,

    /// Optionally rename the header when returning it to the client.
    #[serde(default)]
    pub rename: Option<HeaderName>,

    /// If no subgraph returns the header, set this default value.
    #[serde(default)]
    pub default: Option<String>,

    /// How to merge values across multiple subgraph responses.
    #[serde(default)]
    pub algorithm: Option<AggregationAlgo>,
}
