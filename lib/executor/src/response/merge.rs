use crate::response::value::Value;

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

    target_vec.reserve(source_obj.len());

    for (source_key, source_value) in source_obj {
        if let Some((_, target_value)) = target_vec
            .iter_mut()
            .find(|(target_key, _)| *target_key == source_key)
        {
            deep_merge_internal(target_value, source_value);
        } else {
            target_vec.push((source_key, source_value));
        }
    }
}
