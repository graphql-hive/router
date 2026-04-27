use crate::expressions::FromVrlValue;
use vrl::core::Value as VrlValue;

/// Error type for Boolean conversion failures
#[derive(Debug, thiserror::Error, Clone)]
pub enum BooleanConversionError {
    #[error("Cannot convert {type_name} to boolean")]
    UnsupportedType { type_name: String },
}

impl FromVrlValue for bool {
    type Error = BooleanConversionError;

    #[inline]
    fn from_vrl_value(value: VrlValue) -> Result<Self, Self::Error> {
        match value {
            VrlValue::Bytes(b) => Ok(b == "true"),
            VrlValue::Integer(i) => Ok(i != 0),
            VrlValue::Float(f) => Ok(f != 0.0),
            VrlValue::Boolean(b) => Ok(b),
            VrlValue::Null => Ok(false),
            other => Err(BooleanConversionError::UnsupportedType {
                type_name: other.kind().to_string(),
            }),
        }
    }
}
