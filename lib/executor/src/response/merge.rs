use crate::response::value::Value;

pub fn deep_merge<'a>(target: &mut Value<'a>, source: Value<'a>) {
    deep_merge_internal(target, source)
}

fn deep_merge_internal<'a>(target: &mut Value<'a>, source: Value<'a>) {
    match (target, source) {
        // Neither an unanswered slot nor a `null` the subgraph answered clears what another
        // fetch already put there.
        (_, Value::Absent | Value::Null) => {}

        // Both objects. Two responses that land in the same position were deserialized
        // against the same shape, so slot `i` means the same field on both sides and the
        // merge is a positional walk: no allocation, no key comparisons, and nothing the
        // source did not mention is touched or copied.
        (Value::Object(target_slots), Value::Object(source_slots)) => {
            for (target_slot, source_value) in target_slots.iter_mut().zip(source_slots) {
                deep_merge_internal(target_slot, source_value);
            }
        }

        // Both are Arrays: merge them element-wise.
        (Value::Array(target_arr), Value::Array(source_arr)) => {
            for (target_val, source_val) in target_arr.iter_mut().zip(source_arr) {
                deep_merge(target_val, source_val);
            }
        }

        // Fallback: the types don't match, or the target is not a container.
        (target_val, source_val) => {
            *target_val = source_val;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj(slots: &[Option<i64>]) -> Value<'static> {
        Value::Object(
            slots
                .iter()
                .map(|slot| match slot {
                    Some(n) => Value::I64(*n),
                    None => Value::Null,
                })
                .collect(),
        )
    }

    fn slots(value: &Value<'_>) -> Vec<Option<i64>> {
        value
            .as_object()
            .unwrap()
            .iter()
            .map(|v| match v {
                Value::I64(n) => Some(*n),
                Value::Null => None,
                other => panic!("unexpected {other:?}"),
            })
            .collect()
    }

    #[test]
    fn a_source_slot_fills_the_matching_target_slot() {
        let mut target = obj(&[Some(1), None, Some(3)]);
        deep_merge(&mut target, obj(&[None, Some(2), None]));
        assert_eq!(slots(&target), [Some(1), Some(2), Some(3)]);
    }

    #[test]
    fn a_source_slot_overwrites_a_filled_target_slot() {
        let mut target = obj(&[Some(1), Some(2)]);
        deep_merge(&mut target, obj(&[Some(10), None]));
        assert_eq!(slots(&target), [Some(10), Some(2)]);
    }

    #[test]
    fn an_empty_source_slot_never_clears_the_target() {
        let mut target = obj(&[Some(1), Some(2)]);
        deep_merge(&mut target, obj(&[None, None]));
        assert_eq!(slots(&target), [Some(1), Some(2)]);
    }

    #[test]
    fn nested_objects_merge_recursively() {
        let mut target = Value::Object(vec![obj(&[Some(1), None])]);
        deep_merge(&mut target, Value::Object(vec![obj(&[None, Some(2)])]));
        let inner = target.slot(0).unwrap();
        assert_eq!(slots(inner), [Some(1), Some(2)]);
    }

    #[test]
    fn arrays_merge_element_wise() {
        let mut target = Value::Array(vec![obj(&[Some(1), None]), obj(&[Some(3), None])]);
        deep_merge(
            &mut target,
            Value::Array(vec![obj(&[None, Some(2)]), obj(&[None, Some(4)])]),
        );
        match &target {
            Value::Array(items) => {
                assert_eq!(slots(&items[0]), [Some(1), Some(2)]);
                assert_eq!(slots(&items[1]), [Some(3), Some(4)]);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn a_shorter_source_leaves_the_remaining_target_slots_alone() {
        // A partial response still zips: `zip` stops at the shorter side.
        let mut target = obj(&[Some(1), Some(2), Some(3)]);
        deep_merge(&mut target, Value::Object(vec![Value::I64(10)]));
        assert_eq!(slots(&target), [Some(10), Some(2), Some(3)]);
    }
}
