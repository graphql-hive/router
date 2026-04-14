use std::cmp::Ordering;

use crate::response::value::{Value, ValueObject};

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
        (Value::Object(target_obj), Value::Object(source_obj)) => {
            deep_merge_objects(target_obj, source_obj);
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

fn deep_merge_objects<'a>(target_obj: &mut ValueObject<'a>, source_obj: ValueObject<'a>) {
    if source_obj.is_empty() {
        return;
    }
    if target_obj.is_empty() {
        target_obj.clear();
        target_obj.extend(source_obj);

        return;
    }

    let old_target = std::mem::take(target_obj);
    let mut merged = ValueObject::with_capacity(old_target.len() + source_obj.len());

    let mut target_iter = old_target.into_iter().peekable();
    let mut source_iter = source_obj.into_iter().peekable();

    while let (Some(&(target_key, _)), Some(&(source_key, _))) =
        (target_iter.peek(), source_iter.peek())
    {
        match target_key.cmp(source_key) {
            Ordering::Less => {
                merged.push(target_iter.next().unwrap());
            }
            Ordering::Greater => {
                let (key, value) = source_iter.next().unwrap();
                merged.push((key, value));
            }
            Ordering::Equal => {
                let (key, target_val_ref) = target_iter.next().unwrap();
                let (_, source_val_ref) = source_iter.next().unwrap();

                let mut new_val = target_val_ref;
                deep_merge_internal(&mut new_val, source_val_ref);
                merged.push((key, new_val));
            }
        }
    }

    // At this point, at least one of the iterators is exhausted.
    // We can extend the merged vector with the remaining elements from both.
    // For the exhausted iterator, this will be a no-op.
    merged.extend(target_iter);
    merged.extend(source_iter);

    // Replace the original vector with the newly merged one.
    *target_obj = merged;
}
