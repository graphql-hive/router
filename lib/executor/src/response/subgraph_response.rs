use core::fmt;
use std::sync::Arc;

use crate::{
    executors::error::SubgraphExecutorError,
    response::{graphql_error::GraphQLError, value::Value},
};
use bytes::Bytes;
use futures::stream::BoxStream;
use futures_util::stream;
use http::HeaderMap;
use serde::de::{self, Deserializer, MapAccess, Visitor};

#[derive(Debug, Default)]
pub struct SubgraphResponse<'a> {
    pub data: Value<'a>,
    pub errors: Option<Vec<GraphQLError>>,
    pub extensions: Option<Value<'a>>,
    pub headers: Option<Arc<HeaderMap>>,
    pub bytes: Option<Bytes>,
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

impl GraphQLError {
    pub fn to_subgraph_response(self, subgraph_name: &str) -> SubgraphResponse<'static> {
        // error.add_subgraph_name converts the str it to an owned String. So the resulting GraphQLError doesn't
        // actually borrow subgraph_name - it owns a copy, the rest of the default data is also owned. so
        // technically it is a SubgraphResponse<'static> because nothing inside it actually borrows from outside
        let error_with_subgraph_name = self.add_subgraph_name(subgraph_name);
        SubgraphResponse {
            errors: Some(vec![error_with_subgraph_name]),
            ..Default::default()
        }
    }
}

impl<'a> SubgraphResponse<'a> {
    pub fn deserialize_from_bytes(
        bytes: Bytes,
    ) -> Result<SubgraphResponse<'static>, SubgraphExecutorError> {
        let bytes_ref: &[u8] = &bytes;

        // SAFETY: The byte slice `bytes_ref` is transmuted to have lifetime `'static`.
        // This is safe because the returned `SubgraphResponse` contains a clone of `bytes`
        // in its `bytes` field. `Bytes` is a reference-counted buffer, so this ensures the
        // underlying data remains alive as long as the `SubgraphResponse` does.
        // The `data` field of `SubgraphResponse` contains values that borrow from this buffer,
        // creating a self-referential struct, which is why `unsafe` is required.
        let bytes_ref: &'static [u8] = unsafe { std::mem::transmute(bytes_ref) };

        sonic_rs::from_slice(bytes_ref)
            .map_err(|e| SubgraphExecutorError::ResponseDeserializationFailure(e.to_string()))
            .map(|mut resp: SubgraphResponse<'static>| {
                resp.bytes = Some(bytes);
                resp
            })
    }
}

impl SubgraphExecutorError {
    pub fn to_subgraph_response(self, subgraph_name: &str) -> SubgraphResponse<'static> {
        let mut graphql_error: GraphQLError = self.into();
        graphql_error.message = "Failed to execute request to subgraph".to_string();
        graphql_error.to_subgraph_response(subgraph_name)
    }
    pub fn stream_once_subgraph_response(
        self,
        subgraph_name: &str,
    ) -> BoxStream<'static, SubgraphResponse<'static>> {
        let subgraph_response = self.to_subgraph_response(subgraph_name);
        Box::pin(stream::once(async move { subgraph_response }))
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
