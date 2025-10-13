use core::fmt;

use std::collections::HashMap;

use ntex::util::Bytes;
use serde::{de, Deserialize, Deserializer};
use sonic_rs::Value;

use crate::plugin_context::PluginContext;
use crate::plugin_context::RouterHttpRequest;
use crate::plugin_trait::EndHookPayload;
use crate::plugin_trait::EndHookResult;
use crate::plugin_trait::StartHookPayload;
use crate::plugin_trait::StartHookResult;

#[derive(Debug, Clone, Default)]
pub struct GraphQLParams {
    pub query: Option<String>,
    pub operation_name: Option<String>,
    pub variables: HashMap<String, Value>,
    // TODO: We don't use extensions yet, but we definitely will in the future.
    #[allow(dead_code)]
    pub extensions: Option<HashMap<String, Value>>,
}

// Workaround for https://github.com/cloudwego/sonic-rs/issues/114

impl<'de> Deserialize<'de> for GraphQLParams {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct GraphQLErrorExtensionsVisitor;

        impl<'de> de::Visitor<'de> for GraphQLErrorExtensionsVisitor {
            type Value = GraphQLParams;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a map for GraphQLErrorExtensions")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: de::MapAccess<'de>,
            {
                let mut query = None;
                let mut operation_name = None;
                let mut variables: Option<HashMap<String, Value>> = None;
                let mut extensions: Option<HashMap<String, Value>> = None;
                let mut extra_params = HashMap::new();

                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "query" => {
                            if query.is_some() {
                                return Err(de::Error::duplicate_field("query"));
                            }
                            query = map.next_value::<Option<String>>()?;
                        }
                        "operationName" => {
                            if operation_name.is_some() {
                                return Err(de::Error::duplicate_field("operationName"));
                            }
                            operation_name = map.next_value::<Option<String>>()?;
                        }
                        "variables" => {
                            if variables.is_some() {
                                return Err(de::Error::duplicate_field("variables"));
                            }
                            variables = map.next_value::<Option<HashMap<String, Value>>>()?;
                        }
                        "extensions" => {
                            if extensions.is_some() {
                                return Err(de::Error::duplicate_field("extensions"));
                            }
                            extensions = map.next_value::<Option<HashMap<String, Value>>>()?;
                        }
                        other => {
                            let value: Value = map.next_value()?;
                            extra_params.insert(other.to_string(), value);
                        }
                    }
                }

                Ok(GraphQLParams {
                    query,
                    operation_name,
                    variables: variables.unwrap_or_default(),
                    extensions,
                })
            }
        }

        deserializer.deserialize_map(GraphQLErrorExtensionsVisitor)
    }
}

pub struct OnGraphQLParamsStartHookPayload<'exec> {
    pub router_http_request: &'exec RouterHttpRequest<'exec>,
    pub context: &'exec PluginContext,
    pub body: Bytes,
    pub graphql_params: Option<GraphQLParams>,
}

impl<'exec> StartHookPayload<OnGraphQLParamsEndHookPayload<'exec>>
    for OnGraphQLParamsStartHookPayload<'exec>
{
}

pub type OnGraphQLParamsStartHookResult<'exec> = StartHookResult<
    'exec,
    OnGraphQLParamsStartHookPayload<'exec>,
    OnGraphQLParamsEndHookPayload<'exec>,
>;

pub struct OnGraphQLParamsEndHookPayload<'exec> {
    pub graphql_params: GraphQLParams,
    pub context: &'exec PluginContext,
}

impl<'exec> EndHookPayload for OnGraphQLParamsEndHookPayload<'exec> {}

pub type OnGraphQLParamsEndHookResult<'exec> = EndHookResult<OnGraphQLParamsEndHookPayload<'exec>>;
