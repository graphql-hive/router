use std::string::FromUtf8Error;

use crate::expressions::{FromVrlValue, ValueOrProgram};
use vrl::core::Value as VrlValue;

/// Type alias for a Boolean that can be either static or computed via expression
pub type BooleanOrProgram = ValueOrProgram<bool>;

/// Error type for String conversion failures
#[derive(Debug, thiserror::Error, Clone)]
pub enum StringConversionError {
    #[error("Failed to convert bytes to UTF-8 string: {0}")]
    InvalidUtf8(#[from] FromUtf8Error),

    #[error("Cannot convert {type_name} to boolean")]
    UnsupportedType { type_name: String },
}

impl FromVrlValue for bool {
    type Error = StringConversionError;

    #[inline]
    fn from_vrl_value(value: VrlValue) -> Result<Self, Self::Error> {
        match value {
            VrlValue::Bytes(b) => Ok(b == "true"),
            VrlValue::Integer(i) => Ok(i != 0),
            VrlValue::Float(f) => Ok(f != 0.0),
            VrlValue::Boolean(b) => Ok(b),
            VrlValue::Null => Ok(false),
            other => Err(StringConversionError::UnsupportedType {
                type_name: other.kind().to_string(),
            }),
        }
    }
}
