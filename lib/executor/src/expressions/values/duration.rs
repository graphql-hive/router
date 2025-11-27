use crate::expressions::{FromVrlValue, ValueOrProgram};
use humantime::{parse_duration, DurationError};
use std::{str::Utf8Error, time::Duration};
use vrl::core::Value as VrlValue;

/// Type alias for a Duration that can be either static or computed via expression
pub type DurationOrProgram = ValueOrProgram<Duration>;

#[derive(Debug, thiserror::Error, Clone)]
pub enum DurationParseErrorSource {
    #[error("Invalid UTF-8 encoding in duration string: {0}")]
    Utf8(#[from] Utf8Error),
    #[error("Invalid duration format: {0}")]
    Humantime(#[from] DurationError),
}

#[derive(Debug, thiserror::Error, Clone)]
pub enum DurationConversionError {
    #[error("Duration cannot be negative")]
    NegativeValue,

    #[error("Invalid duration type: {type_name}. Expected a non-negative integer (milliseconds) or a duration string (e.g., '30s', '5m', '1h')")]
    UnexpectedType { type_name: String },

    #[error(transparent)]
    ParseError(#[from] DurationParseErrorSource),
}

impl From<Utf8Error> for DurationConversionError {
    fn from(err: Utf8Error) -> Self {
        DurationConversionError::ParseError(DurationParseErrorSource::Utf8(err))
    }
}

impl From<DurationError> for DurationConversionError {
    fn from(err: DurationError) -> Self {
        DurationConversionError::ParseError(DurationParseErrorSource::Humantime(err))
    }
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
                let s = std::str::from_utf8(&b)?;
                Ok(parse_duration(s)?)
            }
            other => Err(DurationConversionError::UnexpectedType {
                type_name: other.kind().to_string(),
            }),
        }
    }
}
