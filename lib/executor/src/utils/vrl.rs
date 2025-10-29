use std::collections::BTreeMap;

use bytes::Bytes;
use ordered_float::NotNan;
use sonic_rs::{JsonContainerTrait, JsonValueTrait};
use vrl::core::Value;

pub fn sonic_value_to_vrl_value(value: &sonic_rs::Value) -> vrl::core::Value {
    if value.is_null() {
        return Value::Null;
    }

    if let Some(b) = value.as_bool() {
        return Value::Boolean(b);
    }

    if let Some(s) = value.as_str() {
        return Value::Bytes(Bytes::from(s.to_string()));
    }

    if let Some(i) = value.as_i64() {
        return Value::Integer(i);
    }

    if let Some(u) = value.as_u64() {
        // Note: This can overflow if the u64 value is larger than i64::MAX.
        // VRL uses i64 for integers, so a choice has to be made.
        // For now, we cast, accepting the risk of overflow. A more robust
        // implementation might convert to a float, a string, or return an error.
        return Value::Integer(u as i64);
    }

    if let Some(f) = value.as_f64() {
        return Value::Float(NotNan::new(f).unwrap());
    }

    if let Some(arr) = value.as_array() {
        let v = arr.iter().map(sonic_value_to_vrl_value).collect();
        return Value::Array(v);
    }

    if let Some(obj) = value.as_object() {
        let mut map = BTreeMap::new();

        for (k, v) in obj.iter() {
            map.insert(k.into(), sonic_value_to_vrl_value(v));
        }

        return Value::Object(map);
    }

    Value::Null
}
