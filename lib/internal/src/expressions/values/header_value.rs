use crate::expressions::FromVrlValue;
use http::HeaderValue;
use vrl::core::Value as VrlValue;

/// Error type for HeaderValue conversion failures
#[derive(Debug, thiserror::Error, Clone)]
pub enum HeaderValueConversionError {
    #[error("VRL Array cannot be converted to header value")]
    UnsupportedArray,

    #[error("VRL Regex cannot be converted to header value")]
    UnsupportedRegex,

    #[error("VRL Timestamp cannot be converted to header value")]
    UnsupportedTimestamp,

    #[error("VRL Object cannot be converted to header value")]
    UnsupportedObject,

    #[error("VRL Null cannot be converted to header value")]
    UnsupportedNull,

    #[error("Invalid header value: {reason}")]
    InvalidHeaderValue { reason: String },
}

impl FromVrlValue for HeaderValue {
    type Error = HeaderValueConversionError;

    #[inline]
    fn from_vrl_value(value: VrlValue) -> Result<Self, Self::Error> {
        match value {
            VrlValue::Bytes(bytes) => HeaderValue::from_bytes(&bytes).map_err(|e| {
                HeaderValueConversionError::InvalidHeaderValue {
                    reason: e.to_string(),
                }
            }),
            VrlValue::Float(f) => HeaderValue::from_str(&f.to_string()).map_err(|e| {
                HeaderValueConversionError::InvalidHeaderValue {
                    reason: e.to_string(),
                }
            }),
            VrlValue::Boolean(b) => {
                let s = if b { "true" } else { "false" };
                HeaderValue::from_str(s).map_err(|e| {
                    HeaderValueConversionError::InvalidHeaderValue {
                        reason: e.to_string(),
                    }
                })
            }
            VrlValue::Integer(i) => HeaderValue::from_str(&i.to_string()).map_err(|e| {
                HeaderValueConversionError::InvalidHeaderValue {
                    reason: e.to_string(),
                }
            }),
            VrlValue::Array(_) => Err(HeaderValueConversionError::UnsupportedArray),
            VrlValue::Regex(_) => Err(HeaderValueConversionError::UnsupportedRegex),
            VrlValue::Timestamp(_) => Err(HeaderValueConversionError::UnsupportedTimestamp),
            VrlValue::Object(_) => Err(HeaderValueConversionError::UnsupportedObject),
            VrlValue::Null => Err(HeaderValueConversionError::UnsupportedNull),
        }
    }
}
