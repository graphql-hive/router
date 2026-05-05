use std::collections::BTreeMap;

use bytes::Bytes;
use hive_router_internal::expressions::{lib::ToVrlValue, vrl::core::Value};
use http::HeaderValue;

use crate::headers::{request::RequestExpressionContext, response::ResponseExpressionContext};
use hive_router_internal::expressions::FromVrlValue;

/// Convert a VRL value to a HeaderValue, returning an Option
///
/// This function is backward compatible with the old implementation but now
/// uses the FromVrlValue trait internally to provide better error handling.
/// For detailed error information, use `HeaderValue::from_vrl_value()` directly.
pub fn vrl_value_to_header_value(value: Value) -> Option<HeaderValue> {
    HeaderValue::from_vrl_value(value).ok()
}

impl From<&RequestExpressionContext<'_>> for Value {
    /// NOTE: If performance becomes an issue, consider pre-computing parts of this context that do not change
    fn from(ctx: &RequestExpressionContext) -> Self {
        // .subgraph
        let subgraph_value = {
            let value = Self::Bytes(Bytes::from(ctx.subgraph_name.to_owned()));
            Self::Object(BTreeMap::from([("name".into(), value)]))
        };

        // .request
        let request_value: Self = ctx.client_request.into();

        Self::Object(BTreeMap::from([
            ("subgraph".into(), subgraph_value),
            ("request".into(), request_value),
        ]))
    }
}

impl From<&ResponseExpressionContext<'_>> for Value {
    /// NOTE: If performance becomes an issue, consider pre-computing parts of this context that do not change
    fn from(ctx: &ResponseExpressionContext) -> Self {
        // .subgraph
        let subgraph_value = Self::Object(BTreeMap::from([(
            "name".into(),
            Self::Bytes(Bytes::from(ctx.subgraph_name.to_owned())),
        )]));
        // .response
        let response_value = Self::Object(BTreeMap::from([(
            "headers".into(),
            ctx.subgraph_headers.to_vrl_value(),
        )]));
        // .request
        let request_value: Self = ctx.client_request.into();

        Self::Object(BTreeMap::from([
            ("subgraph".into(), subgraph_value),
            ("response".into(), response_value),
            ("request".into(), request_value),
        ]))
    }
}
