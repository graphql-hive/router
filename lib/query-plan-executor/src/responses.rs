use std::cmp::Ordering;

use simd_json::{BorrowedValue, StaticNode};

pub struct ResponsesStorage<'req> {
    // arena: &'req Bump,
    responses: Vec<&'req BorrowedValue<'req>>,
    pub final_response: Value<'req>,
}

impl<'req> ResponsesStorage<'req> {
    pub fn new() -> Self {
        Self {
            // arena,
            responses: Vec::new(),
            final_response: Value::Null,
        }
    }

    pub fn add_response<'sub_req: 'req>(&mut self, response: &'sub_req BorrowedValue<'req>) {
        self.responses.push(response);
        deep_merge_internal(&mut self.final_response, response);
    }
}

pub enum Value<'a> {
    Null,
    Bool(&'a bool),
    F64(&'a f64),
    I64(&'a i64),
    U64(&'a u64),
    String(&'a str),
    Array(Vec<Value<'a>>),
    Object(Vec<(&'a str, Value<'a>)>),
}

impl Value<'_> {
    pub fn as_object(&self) -> Option<&Vec<(&str, Value)>> {
        match self {
            Value::Object(obj) => Some(obj),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }
    pub fn is_null(&self) -> bool {
        match self {
            Value::Null => true,
            _ => false,
        }
    }
}

impl<'a> From<&'a BorrowedValue<'a>> for Value<'a> {
    fn from(borrowed_value: &'a BorrowedValue<'a>) -> Self {
        match borrowed_value {
            BorrowedValue::Static(s) => match s {
                StaticNode::Null => Value::Null,
                StaticNode::Bool(b) => Value::Bool(b),
                StaticNode::F64(f) => Value::F64(f),
                StaticNode::I64(i) => Value::I64(i),
                StaticNode::U64(u) => Value::U64(u),
            },
            BorrowedValue::String(s) => Value::String(s),
            BorrowedValue::Array(arr) => {
                Value::Array(arr.iter().map(|v| v.into()).collect::<Vec<_>>())
            }
            BorrowedValue::Object(obj) => {
                let mut arr = obj
                    .iter()
                    .map(|(k, v)| (k.as_ref(), v.into()))
                    .collect::<Vec<_>>();
                arr.sort_unstable_by_key(|(k, _)| *k);
                Value::Object(arr)
            }
        }
    }
}

fn deep_merge_internal<'a>(target: &mut Value<'a>, source: &'a BorrowedValue<'a>) {
    match (target, source) {
        // If the source value is null, we do nothing.
        (_, BorrowedValue::Static(StaticNode::Null)) => {
            // No-op
        }

        // Both are Objects: merge them using the helper.
        (Value::Object(target_vec), BorrowedValue::Object(source_obj)) => {
            deep_merge_objects(target_vec, source_obj);
        }

        // Both are Arrays: merge them element-wise.
        (Value::Array(target_arr), BorrowedValue::Array(source_arr)) => {
            for (target_val, source_val) in target_arr.iter_mut().zip(source_arr.iter()) {
                deep_merge_internal(target_val, source_val);
            }
        }

        // Fallback: The types don't match, or the target is not a container.
        // Convert the source to a `Value` and replace the target.
        (target_val, source_val) => {
            *target_val = source_val.into();
        }
    }
}

fn deep_merge_objects<'a>(
    target_vec: &mut Vec<(&'a str, Value<'a>)>,
    source_obj: &'a simd_json::value::borrowed::Object<'a>,
) {
    if source_obj.is_empty() {
        return;
    }
    if target_vec.is_empty() {
        *target_vec = source_obj
            .iter()
            .map(|(k, v)| (k.as_ref(), v.into()))
            .collect();
        return;
    }

    // Take ownership of the target vector's contents to perform an efficient merge.
    let old_target = std::mem::take(target_vec);
    let mut merged = Vec::with_capacity(old_target.len() + source_obj.len());

    let mut target_iter = old_target.into_iter().peekable();
    let mut source_iter = source_obj.iter().peekable();

    // Linearly merge while both iterators have elements.
    while let (Some(&(t_key, _)), Some(&(s_key, _))) = (target_iter.peek(), source_iter.peek()) {
        match t_key.cmp(s_key.as_ref()) {
            Ordering::Less => {
                merged.push(target_iter.next().unwrap());
            }
            Ordering::Greater => {
                let (key, value) = source_iter.next().unwrap();
                merged.push((key.as_ref(), value.into()));
            }
            Ordering::Equal => {
                let (_, mut target_value) = target_iter.next().unwrap();
                let (key, source_value) = source_iter.next().unwrap();
                deep_merge_internal(&mut target_value, source_value);
                merged.push((key, target_value));
            }
        }
    }

    // Extend the merged vector with any remaining elements from either iterator.
    merged.extend(target_iter);
    merged.extend(source_iter.map(|(k, v)| (k.as_ref(), v.into())));

    // Replace the original vector with the newly merged one.
    *target_vec = merged;
}
