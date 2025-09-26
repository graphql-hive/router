use std::collections::HashMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

type HeaderName = String;
type RegExp = String;

/// Standard hop-by-hop headers that are never forwarded to subgraphs and are
/// filtered from client responses, regardless of rules.
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

/// Headers that must never be comma-joined. If multiple values exist, they
/// are emitted as separate header fields (e.g. multiple `Set-Cookie` lines).
pub static NEVER_JOIN_HEADERS: &[&str] = &["set-cookie", "www-authenticate"];

/// Configuration for how the Router handles HTTP headers.
///
/// ## Scopes & order of evaluation
/// - **Scope precedence:** Rules under `all` apply to every subgraph first.
///   Rules under `subgraphs.<name>` apply **after** and can override results
///   for that specific subgraph.
/// - **Rule ordering:** Within each list, rules are applied **top-to-bottom**.
///   Later rules can overwrite/undo earlier rules (e.g. `propagate` then `remove`).
///
/// ## Case-insensitive names
/// Header names are case-insensitive. Internally they are normalized to lowercase.
///
/// ## Safety
/// Hop-by-hop headers are always stripped. Never-join headers (e.g. `set-cookie`)
/// are never comma-joined. Multiple values are preserved as separate fields.
///
/// ### Example
/// ```yaml
/// headers:
///   all:
///     request:
///       - propagate:
///           named: Authorization
///       - remove:
///           matching: "^x-legacy-.*"
///       - insert:
///           name: x-router
///           value: hive-router
///
///   subgraphs:
///     accounts:
///       request:
///         - propagate:
///             named: x-tenant-id
///             rename: x-acct-tenant
///             default: unknown
/// ```
#[derive(Debug, Default, Deserialize, Serialize, JsonSchema, Clone)]
pub struct HeadersConfig {
    /// Rules applied to all subgraphs (global defaults).
    #[serde(default)]
    pub all: Option<HeaderRules>,

    /// Rules applied to individual subgraphs.
    /// Keys are subgraph names as defined in the supergraph schema.
    ///
    /// **Precedence:** These are applied **after** `all`, and therefore can
    /// override the result of global rules for that subgraph.
    #[serde(default)]
    pub subgraphs: Option<HashMap<String, HeaderRules>>,
}

/// Rules for a single scope (global or per-subgraph).
///
/// You can specify independent rule lists for **request** (to subgraphs)
/// and **response** (to clients). Within each list, rules are applied in order.
#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, Default)]
pub struct HeaderRules {
    /// Rules that shape the **request** sent from the router to subgraphs.
    #[serde(default)]
    pub request: Option<Vec<RequestHeaderRule>>,

    /// Rules that shape the **response** sent from the router back to the client.
    #[serde(default)]
    pub response: Option<Vec<ResponseHeaderRule>>,
}

/// Request-header rules (applied before sending to a subgraph).
#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(rename_all = "snake_case")]
pub enum RequestHeaderRule {
    /// Forward headers from the client request into the subgraph request.
    ///
    /// - If `rename` is set, the header is forwarded under the new name.
    /// - If **none** of the matched headers exist, `default` is used (when provided).
    ///
    /// **Order matters:** You can propagate first and then `remove` or `insert`
    /// to refine the final output.
    Propagate(RequestPropagateRule),

    /// Remove headers before sending the request to a subgraph.
    ///
    /// Useful to drop sensitive or irrelevant headers, or to undo a previous
    /// `propagate`/`insert`.
    Remove(RemoveRule),

    /// Add or overwrite a header with a static value.
    ///
    /// - For **normal** headers: replaces any existing value.
    /// - For **never-join** headers (e.g. `set-cookie`): **appends** another
    ///   occurrence (multiple lines), never comma-joins.
    Insert(RequestInsertRule),
}

/// Response-header rules (applied before sending back to the client).
#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(rename_all = "snake_case")]
pub enum ResponseHeaderRule {
    /// Forward headers from subgraph responses into the final client response.
    ///
    /// - If multiple subgraphs provide the same header, `algorithm` controls
    ///   how values are merged.
    /// - If **no** subgraph provides a matching header, `default` is used (when provided).
    /// - If `rename` is set, the header is returned under the new name.
    ///
    /// **Never-join headers** (e.g. `set-cookie`) are never comma-joined:
    /// multiple values are returned as separate header fields regardless of `algorithm`.
    Propagate(ResponsePropagateRule),

    /// Remove headers before sending the response to the client.
    Remove(RemoveRule),

    /// Add or overwrite a header in the response to the client.
    ///
    /// For never-join headers, appends another occurrence (multiple lines).
    Insert(ResponseInsertRule),
}

/// Remove headers matched by the specification.
#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
pub struct RemoveRule {
    #[serde(flatten)]
    pub spec: MatchSpec,
}

/// Insert a header with a static value.
///
/// ### Examples
/// ```yaml
/// - insert:
///     name: x-env
///     value: prod
/// ```
///
/// ```yaml
/// - insert:
///     name: set-cookie
///     value: "a=1; Path=/"
/// # If another Set-Cookie exists, this creates another header line (never joined)
/// ```
#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
pub struct RequestInsertRule {
    /// Header name to insert or overwrite (case-insensitive).
    pub name: HeaderName,
    /// Where the value comes from (currently static only).
    #[serde(flatten)]
    pub source: InsertSource,
}

/// Insert a header with a static value.
///
/// ### Examples
/// ```yaml
/// - insert:
///     name: x-env
///     value: prod
/// ```
///
/// ```yaml
/// - insert:
///     name: set-cookie
///     value: "a=1; Path=/"
/// # If another Set-Cookie exists, this creates another header line (never joined)
/// ```
#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
pub struct ResponseInsertRule {
    /// Header name to insert or overwrite (case-insensitive).
    pub name: HeaderName,
    /// Where the value comes from (currently static only).
    #[serde(flatten)]
    pub source: InsertSource,
    /// How to merge values across multiple subgraph responses.
    /// Default: `Last` (overwrite).
    #[serde(default)]
    pub algorithm: Option<AggregationAlgo>,
}

/// Source for an inserted header value.
#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(untagged)]
pub enum InsertSource {
    /// Static value provided in the config.
    Value { value: String },
    /// A dynamic value computed by a VRL expression.
    ///
    /// This allows you to generate header values based on the incoming request,
    /// subgraph name, and (for response rules) subgraph response headers.
    /// The expression has access to a context object with `.request`, `.subgraph`,
    /// and `.response` fields.
    ///
    /// For more information on the available functions and syntax, see the
    /// [VRL documentation](https://vrl.dev/).
    ///
    /// ### Example
    /// ```yaml
    /// # Insert a header with a value derived from another header.
    /// - insert:
    ///     name: x-auth-scheme
    ///     expression: 'split(.request.headers.authorization, " ")[0] ?? "none"'
    /// ```
    Expression { expression: String },
}

/// Helper to allow `one` or `many` values for ergonomics (OR semantics).
///
/// ### Examples
/// ```yaml
/// named: Authorization
/// # or
/// named: [Authorization, x-tenant-id]
/// ```
#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(untagged)]
pub enum OneOrMany<T> {
    One(T),
    Many(Vec<T>),
}

/// Header matching specification used by `propagate` and `remove`.
///
/// **Semantics**
/// - `named`: match by exact name(s), case-insensitive (OR).
/// - `matching`: match header name(s) by regex (OR).
/// - `exclude`: subtract matches by regex (applied **after** `named`/`matching`).
///
/// If `matching` is omitted, it’s treated as “match nothing” unless `named` is set.
/// If both `named` and `matching` are omitted, the rule matches nothing.
///
/// **Safety:** Hop-by-hop headers are never propagated, even if matched here.
///
/// ### Examples
/// ```yaml
/// # Propagate selected exact names
/// named: [Authorization, x-corr-id]
///
/// # Propagate everything starting with x- (except legacy)
/// matching: "^x-.*"
/// exclude: ["^x-legacy-.*"]
/// ```
#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, Default)]
pub struct MatchSpec {
    /// Match headers by exact name (OR).
    #[serde(default)]
    pub named: Option<OneOrMany<HeaderName>>,

    /// Match headers by regex pattern(s) (OR).
    #[serde(default)]
    pub matching: Option<OneOrMany<RegExp>>,

    /// Exclude headers matching these regexes, applied after `matching`.
    #[serde(default)]
    pub exclude: Option<Vec<RegExp>>,
}

/// Propagate headers from the client request to subgraph requests.
///
/// **Behavior**
/// - If `rename` is provided, forwarded under that name.
/// - If **none** of the matched headers are present, `default` (when present)
///   is used under `rename` (if set) or the **first** `named` header.
///
/// ### Examples
/// ```yaml
/// # Forward a specific header, but rename it per subgraph
/// propagate:
///   named: x-tenant-id
///   rename: x-acct-tenant
///
/// # Forward all x- headers except legacy ones
/// propagate:
///   matching: "^x-.*"
///   exclude: ["^x-legacy-.*"]
///
/// # If Authorization is missing, inject a default token for this subgraph
/// propagate:
///   named: Authorization
///   default: "Bearer test-token"
/// ```
#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
pub struct RequestPropagateRule {
    #[serde(flatten)]
    pub spec: MatchSpec,

    /// Optionally rename the header when forwarding.
    #[serde(default)]
    pub rename: Option<HeaderName>,

    /// If the header is missing, set a default value.
    /// Applied only when **none** of the matched headers exist.

    #[serde(default)]
    pub default: Option<String>,
}

/// How to merge response header values from multiple subgraphs.
///
/// - `First`: keep the first value encountered, ignore the rest.
/// - `Last`: overwrite with the last value encountered (**default**).
/// - `Append`: comma-join all values into a single field, **except** for
///   `NEVER_JOIN_HEADERS` which are always emitted as multiple fields.
///
/// **Note:** For never-join headers (e.g. `Set-Cookie`), the router always
/// emits multiple header fields, regardless of the algorithm.
#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum AggregationAlgo {
    /// Take the first value encountered and ignore later ones.
    First,
    /// Overwrite with the last value encountered.
    Last,
    /// Append all values into a comma-separated string (list-valued headers).
    Append,
}

/// Propagate headers from subgraph responses to the final client response.
///
/// **Behavior**
/// - If multiple subgraphs return the header, values are merged using `algorithm`.
///   Never-join headers are **never** comma-joined.
/// - If **no** subgraph returns a match, `default` (if set) is emitted.
/// - If `rename` is set, the outgoing header uses the new name.
///
/// ### Examples
/// ```yaml
/// # Forward Cache-Control from whichever subgraph supplies it (last wins)
/// propagate:
///   named: Cache-Control
///   algorithm: last
///
/// # Combine list-valued headers
/// propagate:
///   named: vary
///   algorithm: append
///
/// # Ensure a fallback header is always present
/// propagate:
///   named: x-backend
///   algorithm: append
///   default: unknown
/// ```
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
    pub algorithm: AggregationAlgo,
}
