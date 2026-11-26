use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, JsonSchema, Default, Clone)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct DeferConfig {
    /// Turns on support for the `@defer` directive. Off by default.
    ///
    /// With it on, `@defer` is part of the public schema, and an operation that uses it gets
    /// an incremental response: the primary part first, then the deferred fragments, sent as
    /// `multipart/mixed`, SSE or WebSocket messages. Both the `incrementalSpec=v0.2` format
    /// and the older `deferSpec=20220824` format are supported, picked by the `Accept` header.
    #[serde(default)]
    pub enabled: bool,
}
