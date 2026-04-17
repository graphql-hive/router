use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt;

use hive_router_internal::json::MapAccessSerdeExt;
use ntex::util::Bytes;
use serde::de;
use serde::de::IgnoredAny;
use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;
use sonic_rs::Value;

use crate::plugin_context::PluginContext;
use crate::plugin_context::RouterHttpRequest;
use crate::plugin_trait::EndHookPayload;
use crate::plugin_trait::EndHookResult;
use crate::plugin_trait::StartHookPayload;
use crate::plugin_trait::StartHookResult;
use ntex::http::Response;

#[derive(Debug, Default, Serialize)]
/// The GraphQL parameters parsed from the HTTP request body by the router.
/// This includes the `query`, `operationName`, `variables`, and `extensions`
/// [Learn more about GraphQL-over-HTTP params](https://graphql.org/learn/serving-over-http/#request-format)
pub struct GraphQLParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    /// The GraphQL query string parsed from the HTTP request body by the router
    /// This contains the source text of a GraphQL query, mutation, or subscription sent by the client in the request body.
    /// It can be `None` if the client did not send a query string in the request body.
    pub query: Option<String>,
    #[serde(rename = "operationName", skip_serializing_if = "Option::is_none")]
    /// The operation name parsed from the HTTP request body by the router
    /// This is the name of the operation that the client wants to execute, sent in the request body.
    /// It is optional and can be `None` if the client did not specify an operation
    pub operation_name: Option<String>,
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    /// The variables map parsed from the HTTP request body by the router
    /// This is a map of variable names to their values sent by the client in the request
    /// [Learn more about GraphQL variables](https://graphql.org/learn/queries/#variables)
    pub variables: HashMap<String, Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extensions: Option<HashMap<String, Value>>,
}

// Workaround for https://github.com/cloudwego/sonic-rs/issues/114

impl<'de> Deserialize<'de> for GraphQLParams {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct GraphQLParamsVisitor;

        impl<'de> de::Visitor<'de> for GraphQLParamsVisitor {
            type Value = GraphQLParams;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a map for GraphQLParams")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: de::MapAccess<'de>,
            {
                let mut query = None;
                let mut operation_name = None;
                let mut variables: Option<HashMap<String, Value>> = None;
                let mut extensions: Option<HashMap<String, Value>> = None;

                while let Some(key) = map.next_key::<Cow<'de, str>>()? {
                    match key.as_ref() {
                        "query" => map.deserialize_once_into_option(&mut query, "query")?,
                        "operationName" => {
                            map.deserialize_once_into_option(&mut operation_name, "operationName")?
                        }
                        "variables" => {
                            map.deserialize_once_into_option(&mut variables, "variables")?
                        }
                        "extensions" => {
                            map.deserialize_once_into_option(&mut extensions, "extensions")?
                        }
                        _ => {
                            let _ = map.next_value::<IgnoredAny>()?;
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

        deserializer.deserialize_map(GraphQLParamsVisitor)
    }
}

pub struct OnGraphQLParamsStartHookPayload<'exec> {
    /// The incoming HTTP request to the router for which the GraphQL execution is happening.
    /// It includes all the details of the request such as headers, body, etc.
    ///
    /// Example:
    /// ```
    ///  let my_header = payload.router_http_request.headers.get("my-header");
    ///  // do something with the header...
    ///  payload.proceed()
    /// ```
    pub router_http_request: &'exec RouterHttpRequest<'exec>,
    /// The context object that can be used to share data across different plugin hooks for the same request.
    /// It is unique per request and is dropped after the response is sent.
    ///
    /// [Learn more about the context data sharing in the docs](https://the-guild.dev/graphql/hive/docs/router/extensibility/plugin_system#context-data-sharing)
    pub context: &'exec PluginContext,
    /// The raw body of the incoming HTTP request.
    /// This is useful for plugins that want to parse the body in a custom way,
    /// or want to access the raw body for logging or other purposes.
    pub body: Bytes,
    /// The overriden GraphQL parameters to be used in the execution instead of the ones parsed from the HTTP request.
    /// If this is `None`, the router will use the GraphQL parameters parsed from the HTTP request.
    /// This is useful for plugins that want to parse the GraphQL parameters in a custom way,
    /// or want to override the GraphQL parameters for testing or other purposes.
    ///
    /// [Learn more about overriding the default behavior](https://the-guild.dev/graphql/hive/docs/router/extensibility/plugin_system#overriding-default-behavior)
    pub graphql_params: Option<GraphQLParams>,
}

impl<'exec> OnGraphQLParamsStartHookPayload<'exec> {
    /// Overrides GraphQL parameters to be used in the execution instead of the ones parsed from the HTTP request.
    /// If this is `None`, the router will use the GraphQL parameters parsed from the HTTP request.
    /// This is useful for plugins that want to parse the GraphQL parameters in a custom way,
    /// or want to override the GraphQL parameters for testing or other purposes.
    ///
    /// [Learn more about overriding the default behavior](https://the-guild.dev/graphql/hive/docs/router/extensibility/plugin_system#overriding-default-behavior)
    pub fn with_graphql_params(mut self, graphql_params: GraphQLParams) -> Self {
        self.graphql_params = Some(graphql_params);
        self
    }
}

impl<'exec> StartHookPayload<OnGraphQLParamsEndHookPayload<'exec>, Response>
    for OnGraphQLParamsStartHookPayload<'exec>
{
}

pub type OnGraphQLParamsStartHookResult<'exec> = StartHookResult<
    'exec,
    OnGraphQLParamsStartHookPayload<'exec>,
    OnGraphQLParamsEndHookPayload<'exec>,
    Response,
>;

pub struct OnGraphQLParamsEndHookPayload<'exec> {
    /// Parsed GraphQL parameters to be used in the execution.
    /// This is either the result of parsing the HTTP request body by the router,
    /// or the overridden GraphQL parameters set by the plugin in the `OnGraphQLParamsStartHookPayload`.
    ///
    /// [Learn more about overriding the default behavior](https://the-guild.dev/graphql/hive/docs/router/extensibility/plugin_system#overriding-default-behavior)
    pub graphql_params: GraphQLParams,
    /// The context object that can be used to share data across different plugin hooks for the same request.
    /// It is unique per request and is dropped after the response is sent.
    ///
    /// [Learn more about the context data sharing in the docs](https://the-guild.dev/graphql/hive/docs/router/extensibility/plugin_system#context-data-sharing)
    pub context: &'exec PluginContext,
}

impl<'exec> EndHookPayload<Response> for OnGraphQLParamsEndHookPayload<'exec> {}

pub type OnGraphQLParamsEndHookResult<'exec> =
    EndHookResult<OnGraphQLParamsEndHookPayload<'exec>, Response>;

#[cfg(test)]
use ntex::web::test;

#[cfg(test)]
impl Into<test::TestRequest> for GraphQLParams {
    fn into(self) -> test::TestRequest {
        let body = self;
        test::TestRequest::post().uri("/graphql").set_json(&body)
    }
}
