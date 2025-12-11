use std::collections::BTreeMap;

use bytes::Bytes;
use hive_router_internal::expressions::vrl::core::Value;
use http::{HeaderMap, HeaderValue};

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

fn header_map_to_vrl_value(headers: &HeaderMap) -> Value {
    let mut obj = BTreeMap::new();
    for (header_name, header_value) in headers.iter() {
        if let Ok(value) = header_value.to_str() {
            obj.insert(
                header_name.as_str().into(),
                Value::Bytes(Bytes::from(value.to_owned())),
            );
        }
    }
    Value::Object(obj)
}

impl From<&RequestExpressionContext<'_, '_>> for Value {
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

impl From<&ResponseExpressionContext<'_, '_>> for Value {
    /// NOTE: If performance becomes an issue, consider pre-computing parts of this context that do not change
    fn from(ctx: &ResponseExpressionContext) -> Self {
        // .subgraph
        let subgraph_value = Self::Object(BTreeMap::from([(
            "name".into(),
            Self::Bytes(Bytes::from(ctx.subgraph_name.to_owned())),
        )]));
        // .response
        let response_value = header_map_to_vrl_value(ctx.subgraph_headers);
        // .request
        let request_value: Self = ctx.client_request.into();

        Self::Object(BTreeMap::from([
            ("subgraph".into(), subgraph_value),
            ("response".into(), response_value),
            ("request".into(), request_value),
        ]))
    }
}
