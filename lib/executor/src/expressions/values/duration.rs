use crate::expressions::{FromVrlValue, ValueOrProgram};
use humantime::parse_duration;
use std::time::Duration;
use vrl::core::Value as VrlValue;

/// Type alias for a Duration that can be either static or computed via expression
pub type DurationOrProgram = ValueOrProgram<Duration>;

#[derive(Debug, thiserror::Error, Clone)]
pub enum DurationConversionError {
    #[error("duration expression resolved to a negative integer")]
    NegativeValue,

    #[error("Duration expression resolved to an unexpected type. Expected non-negative integer (ms) or duration string, got: {type_name}")]
    UnexpectedType { type_name: String },

    #[error("Failed to parse duration string: {reason}")]
    ParseError { reason: String },
}

impl FromVrlValue for Duration {
    type Error = DurationConversionError;

    #[inline]
    fn from_vrl_value(value: VrlValue) -> Result<Self, Self::Error> {
        match value {
            VrlValue::Integer(i) => {
                if i < 0 {
                    return Err(DurationConversionError::NegativeValue);
                }
                Ok(Duration::from_millis(i as u64))
            }
            VrlValue::Bytes(b) => {
                let s =
                    std::str::from_utf8(&b).map_err(|e| DurationConversionError::ParseError {
                        reason: format!("Invalid UTF-8: {}", e),
                    })?;

                parse_duration(s).map_err(|e| DurationConversionError::ParseError {
                    reason: e.to_string(),
                })
            }
            other => Err(DurationConversionError::UnexpectedType {
                type_name: other.kind().to_string(),
            }),
        }
    }
}
