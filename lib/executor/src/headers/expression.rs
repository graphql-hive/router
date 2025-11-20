use std::collections::BTreeMap;

use bytes::Bytes;
use http::{HeaderMap, HeaderValue};
use tracing::warn;
use vrl::core::Value;

use crate::headers::{request::RequestExpressionContext, response::ResponseExpressionContext};

fn warn_unsupported_conversion_option<T>(type_name: &str) -> Option<T> {
    warn!(
        "Cannot convert VRL {} value to a header value. Please convert it to a string first.",
        type_name
    );

    None
}

pub fn vrl_value_to_header_value(value: Value) -> Option<HeaderValue> {
    match value {
        Value::Bytes(bytes) => HeaderValue::from_bytes(&bytes).ok(),
        Value::Float(f) => HeaderValue::from_str(&f.to_string()).ok(),
        Value::Boolean(b) => HeaderValue::from_str(if b { "true" } else { "false" }).ok(),
        Value::Integer(i) => HeaderValue::from_str(&i.to_string()).ok(),
        Value::Array(_) => warn_unsupported_conversion_option("Array"),
        Value::Regex(_) => warn_unsupported_conversion_option("Regex"),
        Value::Timestamp(_) => warn_unsupported_conversion_option("Timestamp"),
        Value::Object(_) => warn_unsupported_conversion_option("Object"),
        Value::Null => {
            warn!("Cannot convert VRL Null value to a header value.");
            None
        }
    }
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
