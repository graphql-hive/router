use std::borrow::Cow;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
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

impl<T: JsonSchema> JsonSchema for SingleOrMultiple<T> {
    fn schema_name() -> Cow<'static, str> {
        format!("SingleOrMultiple<{}>", T::schema_name()).into()
    }

    fn json_schema(gen: &mut SchemaGenerator) -> Schema {
        let schema = json_schema!({
            "anyOf": [
                gen.subschema_for::<T>(),
                gen.subschema_for::<Vec<T>>()
            ]
        });
        schema
    }
}