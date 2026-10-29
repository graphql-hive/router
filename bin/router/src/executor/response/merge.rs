use crate::executor::response::value::Value;

pub fn deep_merge<'target, 'source: 'target>(target: &mut Value<'target>, source: Value<'source>) {
    deep_merge_internal(target, source)
}

fn deep_merge_internal<'target, 'source: 'target>(
    target: &mut Value<'target>,
    source: Value<'source>,
) {
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

fn deep_merge_objects<'target, 'source: 'target>(
    target_vec: &mut Vec<(&'target str, Value<'target>)>,
    source_obj: Vec<(&'source str, Value<'source>)>,
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

pub fn deep_merge_from<'target, 'source: 'target>(
    target: &mut Value<'target>,
    source: &Value<'source>,
) {
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

fn deep_merge_objects_from<'target, 'source: 'target>(
    target_vec: &mut Vec<(&'target str, Value<'target>)>,
    source_obj: &[(&'source str, Value<'source>)],
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

/// Merges `_entities` results into the spots that asked for them.
/// Most entities land in more than one spot, so every spot gets merged from a borrow
/// except the last one, which gets the entity itself.
pub struct EntityMerger<'e, 'exec> {
    entities: &'e mut [Value<'exec>],
    remaining_uses: Vec<u32>,
}

impl<'e, 'exec> EntityMerger<'e, 'exec> {
    /// `uses` is the entity index of every spot, in the order they get merged.
    /// Counting more uses than really happen is fine, the last one just gets merged from a borrow.
    pub fn new(entities: &'e mut [Value<'exec>], uses: impl IntoIterator<Item = usize>) -> Self {
        let mut remaining_uses = vec![0; entities.len()];
        for index in uses {
            if let Some(remaining) = remaining_uses.get_mut(index) {
                *remaining += 1;
            }
        }
        Self {
            entities,
            remaining_uses,
        }
    }

    #[inline]
    pub fn merge_into<'target>(&mut self, target: &mut Value<'target>, entity_index: usize)
    where
        'exec: 'target,
    {
        let (Some(entity), Some(remaining)) = (
            self.entities.get_mut(entity_index),
            self.remaining_uses.get_mut(entity_index),
        ) else {
            return;
        };
        let last_use = remaining.checked_sub(1).is_some_and(|left| {
            *remaining = left;
            left == 0
        });
        if last_use {
            deep_merge(target, std::mem::take(entity));
        } else {
            deep_merge_from(target, entity);
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
    fn entity_merger_moves_each_entity_on_its_last_use() {
        let mut entities = vec![parse(r#"{"id": "a"}"#), parse(r#"{"id": "b"}"#)];
        // "a" goes to two targets, "b" to one, and one extra use of "b" never happens.
        let mut merger = super::EntityMerger::new(&mut entities, [0, 1, 0, 1]);

        let id = |value: &Value| {
            Value::object_get(value.as_object()?, "id")?
                .as_str()
                .map(String::from)
        };
        let mut targets = vec![parse("{}"), parse("{}"), parse("{}")];
        merger.merge_into(&mut targets[0], 0);
        merger.merge_into(&mut targets[1], 1);
        merger.merge_into(&mut targets[2], 0);
        merger.merge_into(&mut parse("{}"), 2);
        drop(merger);

        assert_eq!(id(&targets[0]).as_deref(), Some("a"));
        assert_eq!(id(&targets[1]).as_deref(), Some("b"));
        assert_eq!(id(&targets[2]).as_deref(), Some("a"));
        // "a" was used up, so it was moved out. "b" still has a use left, so it was borrowed.
        assert!(entities[0].is_null());
        assert!(entities[1].is_object());
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
