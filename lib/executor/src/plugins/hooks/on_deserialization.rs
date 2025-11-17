use std::collections::HashMap;

use ntex::util::Bytes;
use serde::Deserialize;
use serde::Deserializer;
use sonic_rs::Value;

use crate::plugin_trait::EndPayload;
use crate::plugin_trait::StartPayload;

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct GraphQLParams {
    pub query: Option<String>,
    pub operation_name: Option<String>,
    #[serde(default, deserialize_with = "deserialize_null_default")]
    pub variables: HashMap<String, Value>,
    // TODO: We don't use extensions yet, but we definitely will in the future.
    #[allow(dead_code)]
    pub extensions: Option<HashMap<String, Value>>,
}

fn deserialize_null_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    T: Default + Deserialize<'de>,
    D: Deserializer<'de>,
{
    let opt = Option::<T>::deserialize(deserializer)?;
    Ok(opt.unwrap_or_default())
}

pub struct OnDeserializationStartPayload<'exec> {
    pub router_http_request: &'exec ntex::web::HttpRequest,
    pub body: Bytes,
    pub graphql_params: Option<GraphQLParams>,
}

impl<'exec> StartPayload<OnDeserializationEndPayload<'exec>> for OnDeserializationStartPayload<'exec> {}

pub struct OnDeserializationEndPayload<'exec> {
    pub router_http_request: &'exec ntex::web::HttpRequest,
    pub graphql_params: GraphQLParams,
}

impl<'exec> EndPayload for OnDeserializationEndPayload<'exec> {}
