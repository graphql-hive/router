use std::hash::{Hash, Hasher};
use xxhash_rust::xxh3::Xxh3;

use crate::parser::query::{Directive, Number, Type, Value};

pub fn hash_list_unordered<TIterator: Iterator>(items: TIterator) -> u64
where
    TIterator::Item: Hash,
{
    let mut xor = 0u64;
    let mut sum = 0u64;
    let mut count = 0u64;
    for item in items {
        let mut hasher = Xxh3::new();
        item.hash(&mut hasher);
        let value = hasher.finish();
        xor ^= value;
        sum = sum.wrapping_add(value);
        count = count.wrapping_add(1);
    }
    let mut hasher = Xxh3::new();
    xor.hash(&mut hasher);
    sum.hash(&mut hasher);
    count.hash(&mut hasher);
    hasher.finish()
}

impl Hash for Directive<'_, String> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        "Directive".hash(state);
        self.name.hash(state);
        hash_list_unordered(self.arguments.iter()).hash(state);
    }
}

impl Hash for Value<'_, String> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        match self {
            Value::Variable(v) => {
                "Value::Variable".hash(state);
                v.hash(state);
            }
            Value::Int(i) => {
                "Value::Int".hash(state);
                i.hash(state);
            }
            Value::Float(f) => {
                "Value::Float".hash(state);
                // We need to convert the float to a canonical form before hashing it,
                // because different representations of the same number (e.g. 1.0 and 1.00)
                // should hash to the same value.
                let canonical = if f.is_nan() {
                    f64::NAN
                } else if f.is_infinite() {
                    if f.is_sign_positive() {
                        f64::INFINITY
                    } else {
                        f64::NEG_INFINITY
                    }
                } else {
                    *f
                };
                canonical.to_bits().hash(state);
            }
            Value::String(s) => {
                "Value::String".hash(state);
                s.hash(state);
            }
            Value::Boolean(b) => {
                "Value::Boolean".hash(state);
                b.hash(state);
            }
            Value::Null => {
                "Value::Null".hash(state);
            }
            Value::Enum(v) => {
                "Value::Enum".hash(state);
                v.hash(state);
            }
            Value::List(l) => {
                "Value::List".hash(state);
                l.len().hash(state);
                for item in l {
                    item.hash(state);
                }
            }
            Value::Object(o) => {
                "Value::Object".hash(state);

                hash_list_unordered(o.iter()).hash(state);
            }
        }
    }
}

impl Hash for Type<'_, String> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        match self {
            Type::NamedType(n) => {
                "Type::NamedType".hash(state);
                n.hash(state);
            }
            Type::ListType(t) => {
                "Type::ListType".hash(state);
                t.hash(state);
            }
            Type::NonNullType(t) => {
                "Type::NonNullType".hash(state);
                t.hash(state);
            }
        }
    }
}

impl Hash for Number {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}
