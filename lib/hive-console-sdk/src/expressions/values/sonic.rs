use std::collections::BTreeMap;

use bytes::Bytes;
use sonic_rs::{JsonContainerTrait, JsonValueTrait};
use vrl::core::Value;

use crate::expressions::lib::ToVrlValue;

impl ToVrlValue for sonic_rs::Value {
    fn to_vrl_value(&self) -> Value {
        if self.is_null() {
            return Value::Null;
        }

        if let Some(b) = self.as_bool() {
            return Value::Boolean(b);
        }

        if let Some(s) = self.as_str() {
            return Value::Bytes(Bytes::from(s.to_string()));
        }

        if let Some(i) = self.as_i64() {
            return Value::Integer(i);
        }

        if let Some(u) = self.as_u64() {
            if let Ok(value) = i64::try_from(u) {
                return Value::Integer(value);
            }

            return Value::from_f64_or_zero(u as f64);
        }

        if let Some(f) = self.as_f64() {
            return Value::from_f64_or_zero(f);
        }

        if let Some(arr) = self.as_array() {
            let v = arr.iter().map(|v| v.to_vrl_value()).collect();
            return Value::Array(v);
        }

        if let Some(obj) = self.as_object() {
            let mut map = BTreeMap::new();

            for (k, v) in obj.iter() {
                map.insert(k.into(), v.to_vrl_value());
            }

            return Value::Object(map);
        }

        Value::Null
    }
}
