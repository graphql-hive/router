use ahash::HashMap;

use crate::response::value::Value;

pub fn deep_merge<'a>(target: &mut Value<'a>, source: Value<'a>) {
    deep_merge_internal(target, source)
}

fn deep_merge_internal<'a>(target: &mut Value<'a>, source: Value<'a>) {
    match (target, source) {
        (_, Value::Null) => {}

        (Value::Object(target_vec), Value::Object(source_obj)) => {
            deep_merge_objects(target_vec, source_obj);
        }

        (Value::Array(target_arr), Value::Array(source_arr)) => {
            for (target_val, source_val) in target_arr.iter_mut().zip(source_arr) {
                deep_merge(target_val, source_val);
            }
        }

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
        *target_vec = source_obj;
        return;
    }

    let lookup: HashMap<&str, usize> = target_vec
        .iter()
        .enumerate()
        .map(|(i, (k, _))| (*k, i))
        .collect();

    for (source_key, source_val) in source_obj {
        if let Some(&idx) = lookup.get(source_key) {
            deep_merge_internal(&mut target_vec[idx].1, source_val);
        } else {
            target_vec.push((source_key, source_val));
        }
    }
}
