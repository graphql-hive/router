use std::borrow::Cow;

use schemars::{json_schema, JsonSchema, Schema, SchemaGenerator};
use serde::de::{DeserializeOwned, Error};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum SingleOrMultiple<T> {
    Single(T),
    Multiple(Vec<T>),
}

impl<'de, T: DeserializeOwned> Deserialize<'de> for SingleOrMultiple<T> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;

        if let Value::Array(_) = &value {
            match serde_json::from_value::<Vec<T>>(value) {
                Ok(multiple) => return Ok(SingleOrMultiple::Multiple(multiple)),
                Err(multiple_err) => {
                    return Err(D::Error::custom(format!(
                        "expected array of items: {}",
                        multiple_err
                    )));
                }
            }
        }

        match serde_json::from_value::<T>(value) {
            Ok(single) => Ok(SingleOrMultiple::Single(single)),
            Err(single_err) => Err(D::Error::custom(format!(
                "expected single value or array, but parsing both failed with error: {}",
                single_err
            ))),
        }
    }
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
