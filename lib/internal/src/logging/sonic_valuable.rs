use std::collections::HashMap;

use sonic_rs::{JsonContainerTrait, JsonNumberTrait, ValueRef};
use valuable::{Listable, Mappable, Valuable, Value, Visit};

/// Zero-allocation bridge from `&sonic_rs::Value` to `valuable::Valuable`.
///
/// Handles all JSON types recursively without any heap allocation:
/// - Null        → `Value::Unit`
/// - Bool        → `Value::Bool`
/// - Number      → `Value::I64` / `Value::U64` / `Value::F64`
/// - String      → `Value::String(&str)`
/// - Array       → `Value::Listable(self)`; `visit` emits one `visit_value` per vec element
/// - Object      → `Value::Mappable(self)`; `visit` emits one `visit_entry` per key-value pair
pub struct SonicValueRef<'a>(pub &'a sonic_rs::Value);

impl<'a> Valuable for SonicValueRef<'a> {
    fn as_value(&self) -> Value<'_> {
        match self.0.as_ref() {
            ValueRef::Null => Value::Unit,
            ValueRef::Bool(b) => Value::Bool(b),
            ValueRef::Number(n) => {
                if let Some(i) = n.as_i64() {
                    Value::I64(i)
                } else if let Some(u) = n.as_u64() {
                    Value::U64(u)
                } else {
                    Value::F64(n.as_f64().unwrap_or(f64::NAN))
                }
            }
            ValueRef::String(s) => Value::String(s),
            ValueRef::Array(_) => Value::Listable(self),
            ValueRef::Object(_) => Value::Mappable(self),
        }
    }

    fn visit(&self, visitor: &mut dyn Visit) {
        match self.0.as_ref() {
            ValueRef::Array(arr) => {
                for item in arr.iter() {
                    visitor.visit_value(SonicValueRef(item).as_value());
                }
            }
            ValueRef::Object(obj) => {
                for (key, val) in obj.iter() {
                    visitor.visit_entry(Value::String(key), SonicValueRef(val).as_value());
                }
            }
            _ => visitor.visit_value(self.as_value()),
        }
    }
}

impl Listable for SonicValueRef<'_> {
    fn size_hint(&self) -> (usize, Option<usize>) {
        match self.0.as_array() {
            Some(arr) => (arr.len(), Some(arr.len())),
            None => (0, Some(0)),
        }
    }
}

impl Mappable for SonicValueRef<'_> {
    fn size_hint(&self) -> (usize, Option<usize>) {
        match self.0.as_object() {
            Some(obj) => (obj.len(), Some(obj.len())),
            None => (0, Some(0)),
        }
    }
}

/// Zero-allocation bridge from `&HashMap<String, sonic_rs::Value>` to `valuable::Valuable`.
///
/// `None` maps are represented as `Value::Unit` via `Option<SonicMapRef>`.
pub struct SonicMapRef<'a>(pub &'a HashMap<String, sonic_rs::Value>);

impl<'a> Valuable for SonicMapRef<'a> {
    fn as_value(&self) -> Value<'_> {
        Value::Mappable(self)
    }

    fn visit(&self, visitor: &mut dyn Visit) {
        for (key, val) in self.0.iter() {
            visitor.visit_entry(Value::String(key.as_str()), SonicValueRef(val).as_value());
        }
    }
}

impl Mappable for SonicMapRef<'_> {
    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.0.len();
        (len, Some(len))
    }
}
