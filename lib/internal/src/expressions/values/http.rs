use std::collections::BTreeMap;

use crate::expressions::{lib::ToVrlValue, FromVrlValue};
use bytes::Bytes;
use http::{HeaderMap, HeaderValue, Uri};
use ntex::http::HeaderMap as NtexHeaderMap;
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

impl ToVrlValue for NtexHeaderMap {
    fn to_vrl_value(&self) -> VrlValue {
        let mut obj = BTreeMap::new();
        for (header_name, header_value) in self.iter() {
            if let Ok(value) = header_value.to_str() {
                obj.insert(
                    header_name.as_str().into(),
                    VrlValue::Bytes(Bytes::from(value.to_owned())),
                );
            }
        }
        VrlValue::Object(obj)
    }
}

impl ToVrlValue for HeaderMap {
    fn to_vrl_value(&self) -> VrlValue {
        let mut obj = BTreeMap::new();
        for (header_name, header_value) in self.iter() {
            if let Ok(value) = header_value.to_str() {
                obj.insert(
                    header_name.as_str().into(),
                    VrlValue::Bytes(Bytes::from(value.to_owned())),
                );
            }
        }
        VrlValue::Object(obj)
    }
}

impl ToVrlValue for Uri {
    fn to_vrl_value(&self) -> VrlValue {
        VrlValue::Object(BTreeMap::from([
            ("host".into(), self.host().unwrap_or("unknown").into()),
            ("path".into(), self.path().into()),
            (
                "port".into(),
                self.port_u16()
                    .unwrap_or_else(|| {
                        if self.scheme() == Some(&http::uri::Scheme::HTTPS) {
                            443
                        } else {
                            80
                        }
                    })
                    .into(),
            ),
        ]))
    }
}
