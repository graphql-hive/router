use core::fmt;
use std::cell::Cell;
use std::sync::Arc;

use hive_router_query_planner::planner::plan_nodes::SchemaInterner;

use crate::response::{
    flat::{FlatResponseData, FlatValueSeed},
    graphql_error::GraphQLError,
};
use bytes::Bytes;
use http::HeaderMap;
use serde::de::{self, Deserializer, MapAccess, Visitor};

thread_local! {
    static ACTIVE_SCHEMA_INTERNER: Cell<*const SchemaInterner> = const { Cell::new(std::ptr::null()) };
}

fn active_schema_interner<'a>() -> Option<&'a SchemaInterner> {
    ACTIVE_SCHEMA_INTERNER.with(|cell| {
        let ptr = cell.get();
        if ptr.is_null() {
            None
        } else {
            // SAFETY: pointer is set by `from_slice_with_schema_interner` for this thread,
            // lives for the whole deserialization call, and reset afterwards.
            Some(unsafe { &*ptr })
        }
    })
}

#[derive(Debug, Default)]
pub struct SubgraphResponse<'a> {
    pub data: FlatResponseData<'a>,
    pub errors: Option<Vec<GraphQLError>>,
    pub extensions: Option<sonic_rs::Value>,
    pub headers: Option<Arc<HeaderMap>>,
    pub bytes: Option<Bytes>,
}

impl<'a> SubgraphResponse<'a> {
    pub fn from_slice_with_schema_interner(
        bytes: &'a [u8],
        schema_interner: &SchemaInterner,
    ) -> Result<Self, sonic_rs::Error> {
        ACTIVE_SCHEMA_INTERNER.with(|cell| {
            let previous = cell.replace(schema_interner as *const SchemaInterner);
            let result = sonic_rs::from_slice(bytes);
            cell.set(previous);
            result
        })
    }
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
                let mut data: Option<FlatResponseData<'de>> = None;
                let mut errors = None;
                let mut extensions = None;

                while let Some(key) = map.next_key::<&str>()? {
                    match key {
                        "data" => {
                            if data.is_some() {
                                return Err(de::Error::duplicate_field("data"));
                            }
                            let mut flat_data = FlatResponseData::default();
                            let root_id = if let Some(schema_interner) = active_schema_interner() {
                                map.next_value_seed(FlatValueSeed::with_schema_interner(
                                    &mut flat_data,
                                    schema_interner,
                                ))?
                            } else {
                                map.next_value_seed(FlatValueSeed::new(&mut flat_data))?
                            };
                            flat_data.set_root(root_id);
                            data = Some(flat_data);
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
                let data = data.unwrap_or_default();

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

#[cfg(test)]
mod tests {
    use hive_router_query_planner::planner::plan_nodes::SchemaInterner;

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

    #[test]
    fn deserialize_response_fails_for_unknown_data_key_in_strict_mode() {
        let json_response = r#"{"data":{"unknown":1}}"#;
        let interner = SchemaInterner::default();
        let err = super::SubgraphResponse::from_slice_with_schema_interner(
            json_response.as_bytes(),
            &interner,
        )
        .unwrap_err();

        assert!(err.to_string().contains("unknown key in data payload"));
    }
}
