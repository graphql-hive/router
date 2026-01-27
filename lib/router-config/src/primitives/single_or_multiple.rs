use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum SingleOrMultiple<T> {
    Single(T),
    Multiple(Vec<T>),
}

impl<T> From<SingleOrMultiple<T>> for Vec<T> {
    fn from(val: SingleOrMultiple<T>) -> Self {
        match val {
            SingleOrMultiple::Single(item) => vec![item],
            SingleOrMultiple::Multiple(items) => items,
        }
    }
}
