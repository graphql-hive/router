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
            // Note: This can overflow if the u64 value is larger than i64::MAX.
            // VRL uses i64 for integers, so a choice has to be made.
            // For now, we cast, accepting the risk of overflow. A more robust
            // implementation might convert to a float, a string, or return an error.
            return Value::Integer(u as i64);
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
