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

/// Same as `deep_merge(target, source.clone())`, but only clones the parts of `source`
/// that end up in `target`. Cloning first would build the source's objects and arrays
/// just to take them apart and throw them away.
pub fn deep_merge_from<'a>(target: &mut Value<'a>, source: &Value<'a>) {
    match (target, source) {
        (_, Value::Null) => {}
        (Value::Object(target_vec), Value::Object(source_obj)) => {
            deep_merge_objects_from(target_vec, source_obj);
        }
        (Value::Array(target_arr), Value::Array(source_arr)) => {
            for (target_val, source_val) in target_arr.iter_mut().zip(source_arr) {
                deep_merge_from(target_val, source_val);
            }
        }
        (target_val, source_val) => *target_val = source_val.clone(),
    }
}

fn deep_merge_objects_from<'a>(
    target_vec: &mut Vec<(&'a str, Value<'a>)>,
    source_obj: &[(&'a str, Value<'a>)],
) {
    if source_obj.is_empty() {
        return;
    }
    if target_vec.is_empty() {
        target_vec.extend_from_slice(source_obj);
        return;
    }

    let mut cursor = 0;
    target_vec.reserve(source_obj.len());
    for (key, source_val) in source_obj {
        match Value::object_position_from(target_vec, key, &mut cursor) {
            Some(index) => deep_merge_from(&mut target_vec[index].1, source_val),
            None => target_vec.push((key, source_val.clone())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{deep_merge, deep_merge_from};
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

    #[test]
    fn deep_merge_from_matches_merging_a_clone() {
        let cases = [
            (
                r#"{"id": "1", "nested": {"x": 1}}"#,
                r#"{"nested": {"y": 2}, "id": "1", "n": 3}"#,
            ),
            (r#"{"b": 1, "a": 2}"#, r#"{"a": 3, "b": null}"#),
            (r#"{}"#, r#"{"a": [1, {"b": 2}]}"#),
            (
                r#"{"a": [{"x": 1}, {"x": 2}]}"#,
                r#"{"a": [{"y": 1}, {"y": 2}, {"y": 3}]}"#,
            ),
            (r#"{"a": {"x": 1}}"#, r#"{"a": "replaced"}"#),
            (r#"{"a": 1}"#, r#"null"#),
            (r#"[1, 2]"#, r#"{"a": 1}"#),
        ];

        for (target, source) in cases {
            let source = parse(source);
            let mut by_clone = parse(target);
            deep_merge(&mut by_clone, source.clone());
            let mut by_ref = parse(target);
            deep_merge_from(&mut by_ref, &source);

            assert_eq!(
                format!("{by_ref:?}"),
                format!("{by_clone:?}"),
                "{target} <- {source:?}"
            );
        }
    }
}
