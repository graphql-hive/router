use crate::executor::response::value::Value;

pub fn deep_merge<'a>(target: &mut Value<'a>, source: Value<'a>) {
    deep_merge_internal(target, source)
}

fn deep_merge_internal<'a>(target: &mut Value<'a>, source: Value<'a>) {
    match (target, source) {
        // If the source value is null, we do nothing.
        (_, Value::Null) => {
            // No-op
        }

        // Both are Objects: merge them using the helper.
        (Value::Object(target_vec), Value::Object(source_obj)) => {
            deep_merge_objects(target_vec, source_obj);
        }

        // Both are Arrays: merge them element-wise.
        (Value::Array(target_arr), Value::Array(source_arr)) => {
            for (target_val, source_val) in target_arr.iter_mut().zip(source_arr) {
                deep_merge(target_val, source_val);
            }
        }

        // Fallback: The types don't match, or the target is not a container.
        // Convert the source to a `Value` and replace the target.
        (target_val, source_val) => {
            *target_val = source_val;
        }
    }
}

fn deep_merge_objects<'a>(
    target_vec: &mut Vec<(&'a str, Value<'a>)>,
    source_obj: Vec<(&'a str, Value<'a>)>,
) {
    if source_obj.is_empty() {
        return;
    }
    if target_vec.is_empty() {
        target_vec.clear();
        target_vec.extend(source_obj);

        return;
    }

    let mut cursor = 0;
    target_vec.reserve(source_obj.len());
    for (key, source_val) in source_obj {
        match Value::object_position_from(target_vec, key, &mut cursor) {
            Some(index) => deep_merge_internal(&mut target_vec[index].1, source_val),
            None => target_vec.push((key, source_val)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::deep_merge;
    use crate::executor::response::value::Value;
    use serde::Deserialize;

    fn parse(json: &'static str) -> Value<'static> {
        let mut deserializer = sonic_rs::Deserializer::from_slice(json.as_bytes());
        Value::deserialize(&mut deserializer).unwrap()
    }

    fn keys<'a>(value: &'a Value<'a>) -> Vec<&'a str> {
        value.as_object().unwrap().iter().map(|(k, _)| *k).collect()
    }

    #[test]
    fn merges_shared_keys_and_appends_new_ones_in_source_order() {
        let mut target = parse(r#"{"id": "1", "name": "a", "nested": {"x": 1}}"#);
        let source = parse(r#"{"price": 10, "nested": {"y": 2}, "id": "1", "stock": 3}"#);

        deep_merge(&mut target, source);

        assert_eq!(keys(&target), ["id", "name", "nested", "price", "stock"]);
        let nested = Value::object_get(target.as_object().unwrap(), "nested").unwrap();
        assert_eq!(keys(nested), ["x", "y"]);
    }

    #[test]
    fn source_scalars_replace_target_values() {
        let mut target = parse(r#"{"b": 1, "a": 2}"#);
        let source = parse(r#"{"a": 3, "b": null}"#);

        deep_merge(&mut target, source);

        let obj = target.as_object().unwrap();
        assert!(matches!(Value::object_get(obj, "a"), Some(Value::U64(3))));
        // A null source value leaves the target untouched.
        assert!(matches!(Value::object_get(obj, "b"), Some(Value::U64(1))));
    }
}
