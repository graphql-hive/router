use std::string::FromUtf8Error;

use crate::expressions::{FromVrlValue, ValueOrProgram};
use vrl::core::Value as VrlValue;

/// Type alias for a String that can be either static or computed via expression
///
/// Useful for endpoints, URLs, or any string configuration that can be dynamic
pub type StringOrProgram = ValueOrProgram<String>;

/// Error type for String conversion failures
#[derive(Debug, thiserror::Error, Clone)]
pub enum StringConversionError {
    #[error("Failed to convert bytes to UTF-8 string: {0}")]
    InvalidUtf8(#[from] FromUtf8Error),

    #[error("Cannot convert {type_name} to string")]
    UnsupportedType { type_name: String },
}

impl FromVrlValue for String {
    type Error = StringConversionError;

    #[inline]
    fn from_vrl_value(value: VrlValue) -> Result<Self, Self::Error> {
        match value {
            VrlValue::Bytes(b) => Ok(String::from_utf8(b.to_vec())?),
            VrlValue::Integer(i) => Ok(i.to_string()),
            VrlValue::Float(f) => Ok(f.to_string()),
            VrlValue::Boolean(b) => Ok(if b { "true" } else { "false" }.to_string()),
            VrlValue::Null => Ok(String::new()),
            other => Err(StringConversionError::UnsupportedType {
                type_name: other.kind().to_string(),
            }),
        }
    }
}
