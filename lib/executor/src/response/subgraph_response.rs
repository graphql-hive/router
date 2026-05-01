use core::fmt;
use std::sync::Arc;

use crate::{
    executors::error::SubgraphExecutorError,
    response::{graphql_error::GraphQLError, value::Value},
};
use bytes::Bytes;
use http::{HeaderMap, StatusCode};
use serde::de::{self, Deserializer, MapAccess, Visitor};

#[derive(Debug, Default)]
pub struct SubgraphResponse<'a> {
    pub data: Value<'a>,
    pub errors: Option<Vec<GraphQLError>>,
    pub extensions: Option<Value<'a>>,
    pub headers: Option<Arc<HeaderMap>>,
    pub bytes: Option<Bytes>,
    pub status: Option<StatusCode>,
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
                            // For errors, deserialize into our new `GraphQLError` struct
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
                let data = data.unwrap_or(Value::Null);

                Ok(SubgraphResponse {
                    data,
                    errors,
                    extensions,
                    status: None,
                    headers: None,
                    bytes: None,
                })
            }
        }

        deserializer.deserialize_map(SubgraphResponseVisitor {
            _marker: std::marker::PhantomData,
        })
    }
}

impl SubgraphResponse<'_> {
    pub fn deserialize_from_bytes(
        bytes: Bytes,
    ) -> Result<SubgraphResponse<'static>, SubgraphExecutorError> {
        let bytes_ref: &[u8] = &bytes;

        // SAFETY: The byte slice `bytes_ref` is transmuted to `'static`.
        // This is safe because the returned `SubgraphResponse` stores the `bytes` (Arc-backed
        // reference-counted buffer) in its `bytes` field, keeping the underlying data alive as
        // long as the `SubgraphResponse` does. The `data` field of `SubgraphResponse` contains
        // values that borrow from this buffer, creating a self-referential struct, which is why
        // `unsafe` is required.
        let bytes_ref: &'static [u8] = unsafe { std::mem::transmute(bytes_ref) };

        sonic_rs::from_slice(bytes_ref)
            .map_err(SubgraphExecutorError::ResponseDeserializationFailure)
            .map(move |mut resp: SubgraphResponse<'static>| {
                // Zero cost of cloning Bytes
                resp.bytes = Some(bytes);
                resp
            })
    }
}

#[cfg(test)]
mod tests {
    // When subgraph returns an error with custom extensions but without `data` field
    #[test]
    fn deserialize_response_without_data_with_errors_with_extensions() {
        let json_response = r#"
        {
            "errors": [
                {
                    "message": "Random error from subgraph",
                    "extensions":{
                        "statusCode": 400
                    }
                }
            ]
        }"#;

        let response: super::SubgraphResponse =
            sonic_rs::from_str(json_response).expect("Failed to deserialize");

        assert!(response.data.is_null());
        let errors = response.errors.as_ref().unwrap();
        insta::assert_snapshot!(sonic_rs::to_string_pretty(&errors).unwrap(), @r###"
        [
          {
            "message": "Random error from subgraph",
            "extensions": {
              "statusCode": 400
            }
          }
        ]"###);
    }
}
