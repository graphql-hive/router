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

/// Configuration for how Router handles HTTP headers.
///
/// This allows you to define rules for which headers should be
/// propagated, removed, or set when requests are sent to subgraphs
/// or responses are sent back to clients.
#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
pub struct HeadersConfig {
    /// Rules applied to all subgraphs (global defaults).
    #[serde(default)]
    pub all: Option<HeaderRules>,

    /// Rules applied to individual subgraphs.
    /// Keys are subgraph names as defined in the supergraph schema.
    #[serde(default)]
    pub subgraphs: Option<HashMap<String, HeaderRules>>,
}

impl Default for HeadersConfig {
    fn default() -> Self {
        Self {
            all: None,
            subgraphs: None,
        }
    }
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
    Set(SetRule),
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(rename_all = "snake_case")]
pub enum ResponseHeaderRule {
    /// Forward headers from subgraph responses into the final client response.
    Propagate(ResponsePropagateRule),
    /// Remove headers before sending the response to the client.
    Remove(RemoveRule),
    /// Add or overwrite a header in the response to the client.
    Set(SetRule),
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
pub struct RemoveRule {
    #[serde(flatten)]
    pub spec: MatchSpec,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
pub struct SetRule {
    pub name: HeaderName,
    #[serde(flatten)]
    pub source: SetSource,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(untagged)]
pub enum SetSource {
    /// Static value provided in the config.
    Value { value: String },
    // Expression { expression: String },
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(rename_all = "snake_case")]
pub enum MatchSpec {
    /// Match a single header by its exact name (header names are normalized to lowercase).
    Named(HeaderName),
    /// Match all headers whose names match the given regular expression.
    /// **Important:** hop-by-hop headers (e.g. `Connection`, `Content-Length` and others)
    /// are **never propagated**, even if the regex matches them.
    /// These headers are stripped automatically by the router for protocol correctness.
    Matching(RegExp),
    /// Match all headers whose names match any of the given regular expressions.
    /// Think of it as OR-ing the regexes (union).
    MatchingAny(Vec<RegExp>),
    /// Match all headers whose names match all of the given regular expressions.
    /// Think of it as AND-ing the regexes (intersection).
    MatchingAll(Vec<RegExp>),
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
    FirstWrite,
    /// Overwrite with the last value encountered (default behavior).
    LastWrite,
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
