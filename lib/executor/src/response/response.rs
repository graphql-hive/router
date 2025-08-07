use core::fmt;

use crate::response::value::Value;
use serde::de::{self, Deserializer, MapAccess, Visitor};
use sonic_rs::LazyValue;

pub struct SubgraphResponse<'a> {
    pub data: Value<'a>,
    pub errors: Option<Vec<LazyValue<'a>>>,
    pub extensions: Option<Value<'a>>,
}

impl<'de> de::Deserialize<'de> for SubgraphResponse<'de> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct SubgraphResponseVisitor<'de> {
            _marker: std::marker::PhantomData<&'de ()>,
        }

        impl<'de> Visitor<'de> for SubgraphResponseVisitor<'de> {
            type Value = SubgraphResponse<'de>;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter
                    .write_str("a GraphQL response object with data, errors, and extensions fields")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut data = None;
                let mut errors = None;
                let mut extensions = None;

                while let Some(key) = map.next_key::<&str>()? {
                    match key {
                        "data" => {
                            if data.is_some() {
                                return Err(de::Error::duplicate_field("data"));
                            }
                            // Let Value's deserializer handle the data field
                            data = Some(map.next_value()?);
                        }
                        "errors" => {
                            if errors.is_some() {
                                return Err(de::Error::duplicate_field("errors"));
                            }
                            // For errors, deserialize directly as Vec<Value>
                            errors = Some(map.next_value()?);
                        }
                        "extensions" => {
                            if extensions.is_some() {
                                return Err(de::Error::duplicate_field("extensions"));
                            }
                            // Let Value's deserializer handle the extensions field
                            extensions = Some(map.next_value()?);
                        }
                        _ => {
                            // Skip unknown fields
                            let _ = map.next_value::<de::IgnoredAny>()?;
                        }
                    }
                }

                // Data field is required in a successful response, but might be null in case of errors
                let data = data.unwrap_or_else(|| Value::Null);

                Ok(SubgraphResponse {
                    data,
                    errors,
                    extensions,
                })
            }
        }

        deserializer.deserialize_map(SubgraphResponseVisitor {
            _marker: std::marker::PhantomData,
        })
    }
}
