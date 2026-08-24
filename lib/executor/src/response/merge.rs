use bumpalo::Bump;

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
            for (target_slot, source_value) in target_slots.iter_mut().zip(source_slots.iter_mut())
            {
                deep_merge_internal(target_slot, std::mem::take(source_value));
            }
        }

        // Both are Arrays: merge them element-wise.
        (Value::Array(target_arr), Value::Array(source_arr)) => {
            for (target_val, source_val) in target_arr.iter_mut().zip(source_arr.iter_mut()) {
                deep_merge_internal(target_val, std::mem::take(source_val));
            }
        }

        // Fallback: the types don't match, or the target is not a container.
        (target_val, source_val) => {
            *target_val = source_val;
        }
    }
}

/// Merges `source` into `target` without taking ownership of it.
///
/// One deduplicated entity is written into every position that asked for it, and cloning the
/// whole entity subtree once per position was the second-hottest symbol in a load profile —
/// more than JSON parsing. Walking the source by reference clones only what the target does
/// not already have, and at a leaf that clone is a 32-byte copy of a value borrowing the
/// response buffer, with no allocation at all.
///
/// Equivalent to `deep_merge(target, source.copy_into(arena))`, minus the copy of everything
/// the target was going to overwrite or already had. `arena` is only touched for the subtrees
/// the target really is missing.
pub fn deep_merge_from_ref<'a>(target: &mut Value<'a>, source: &Value<'a>, arena: &'a Bump) {
    match (target, source) {
        // Neither an unanswered slot nor an answered `null` clears what is already there.
        (_, Value::Absent | Value::Null) => {}

        // Same position, same shape: recurse in place, allocating nothing.
        (Value::Object(target_slots), Value::Object(source_slots)) => {
            for (target_slot, source_value) in target_slots.iter_mut().zip(source_slots.iter()) {
                deep_merge_from_ref(target_slot, source_value, arena);
            }
        }

        (Value::Array(target_items), Value::Array(source_items)) => {
            for (target_item, source_item) in target_items.iter_mut().zip(source_items.iter()) {
                deep_merge_from_ref(target_item, source_item, arena);
            }
        }

        // The target has nothing here, so this subtree does have to be materialized.
        (target_value, source_value) => *target_value = source_value.copy_into(arena),
    }
}

/// Writes one deduplicated entity into one of the targets that asked for it, moving the
/// entity instead of cloning it once no other target is left.
///
/// `remaining[index]` is how many targets still have to receive entity `index`. When an
/// entity feeds a single target -- the common case -- nothing is cloned at all, and the
/// entity list is already empty by the time it is dropped. Cloning entity subtrees and then
/// dropping the originals together cost more than JSON parsing in a load profile.
pub fn merge_entity_into<'a>(
    target: &mut Value<'a>,
    entities: &mut [Value<'a>],
    index: usize,
    remaining: &mut [u32],
    arena: &'a Bump,
) {
    let (Some(count), Some(entity)) = (remaining.get_mut(index), entities.get_mut(index)) else {
        return;
    };

    // A count that ran out means an entity was written into more targets than were counted,
    // and the extra writes would silently merge an emptied entity. The counts come from the
    // same hash lists the traversal reads, so this cannot drift.
    debug_assert!(
        *count > 0,
        "entity {index} written to more targets than counted"
    );

    *count = count.saturating_sub(1);
    if *count == 0 {
        deep_merge(target, std::mem::take(entity));
    } else {
        deep_merge_from_ref(target, entity, arena);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj<'a>(arena: &'a Bump, slots: &[Option<i64>]) -> Value<'a> {
        Value::Object(arena.alloc_slice_fill_iter(slots.iter().map(|slot| match slot {
            Some(n) => Value::I64(*n),
            None => Value::Null,
        })))
    }

    fn wrap<'a>(arena: &'a Bump, items: Vec<Value<'a>>) -> &'a mut [Value<'a>] {
        arena.alloc_slice_fill_iter(items)
    }

    fn slots(value: &Value<'_>) -> Vec<Option<i64>> {
        value
            .as_object()
            .unwrap()
            .iter()
            .map(|v| match v {
                Value::I64(n) => Some(*n),
                Value::Null => None,
                Value::Absent => None,
                other => panic!("unexpected {other:?}"),
            })
            .collect()
    }

    #[test]
    fn an_empty_source_slot_never_clears_the_target() {
        let arena = Bump::new();
        let mut target = obj(&arena, &[Some(1), Some(2)]);
        deep_merge(&mut target, obj(&arena, &[None, None]));
        assert_eq!(slots(&target), [Some(1), Some(2)]);
    }

    #[test]
    fn nested_objects_merge_recursively() {
        let arena = Bump::new();
        let mut target = Value::Object(wrap(&arena, vec![obj(&arena, &[Some(1), None])]));
        deep_merge(
            &mut target,
            Value::Object(wrap(&arena, vec![obj(&arena, &[None, Some(2)])])),
        );
        let inner = target.slot(0).unwrap();
        assert_eq!(slots(inner), [Some(1), Some(2)]);
    }

    #[test]
    fn arrays_merge_element_wise() {
        let arena = Bump::new();
        let mut target = Value::Array(wrap(
            &arena,
            vec![obj(&arena, &[Some(1), None]), obj(&arena, &[Some(3), None])],
        ));
        deep_merge(
            &mut target,
            Value::Array(wrap(
                &arena,
                vec![obj(&arena, &[None, Some(2)]), obj(&arena, &[None, Some(4)])],
            )),
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
    fn merging_from_a_reference_matches_merging_an_owned_copy() {
        // The whole point of `deep_merge_from_ref` is to avoid materializing the source, so it
        // has to land in exactly the same place a copy would have.
        let arena = Bump::new();
        let cases: Vec<(Value, Value)> = vec![
            (
                obj(&arena, &[Some(1), None, Some(3)]),
                obj(&arena, &[None, Some(2), None]),
            ),
            (obj(&arena, &[Some(1), Some(2)]), obj(&arena, &[Some(10), None])),
            (
                Value::Object(wrap(&arena, vec![obj(&arena, &[Some(1), None])])),
                Value::Object(wrap(&arena, vec![obj(&arena, &[None, Some(2)])])),
            ),
            (
                Value::Array(wrap(&arena, vec![obj(&arena, &[Some(1), None])])),
                Value::Array(wrap(&arena, vec![obj(&arena, &[None, Some(2)])])),
            ),
            (Value::Absent, obj(&arena, &[Some(7)])),
            (obj(&arena, &[Some(1)]), Value::Null),
        ];

        for (target, source) in cases {
            let mut by_value = target.copy_into(&arena);
            deep_merge(&mut by_value, source.copy_into(&arena));

            let mut by_ref = target.copy_into(&arena);
            deep_merge_from_ref(&mut by_ref, &source, &arena);

            assert_eq!(
                format!("{by_value:?}"),
                format!("{by_ref:?}"),
                "diverged for target={target:?} source={source:?}"
            );
        }
    }

    #[test]
    fn a_shorter_source_leaves_the_remaining_target_slots_alone() {
        // A partial response still zips: `zip` stops at the shorter side.
        let arena = Bump::new();
        let mut target = obj(&arena, &[Some(1), Some(2), Some(3)]);
        deep_merge(
            &mut target,
            Value::Object(wrap(&arena, vec![Value::I64(10)])),
        );
        assert_eq!(slots(&target), [Some(10), Some(2), Some(3)]);
    }
}
