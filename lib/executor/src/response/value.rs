use simd_json::{BorrowedValue, StaticNode};

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
