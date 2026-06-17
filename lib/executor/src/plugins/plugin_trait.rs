use crate::{
    hooks::{
        on_execute::{OnExecuteStartHookPayload, OnExecuteStartHookResult},
        on_graphql_error::{OnGraphQLErrorHookPayload, OnGraphQLErrorHookResult},
        on_graphql_params::{OnGraphQLParamsStartHookPayload, OnGraphQLParamsStartHookResult},
        on_graphql_parse::{OnGraphQLParseHookResult, OnGraphQLParseStartHookPayload},
        on_graphql_validation::{
            OnGraphQLValidationStartHookPayload, OnGraphQLValidationStartHookResult,
        },
        on_http_request::{OnHttpRequestHookPayload, OnHttpRequestHookResult},
        on_plugin_init::{OnPluginInitPayload, OnPluginInitResult},
        on_query_plan::{OnQueryPlanStartHookPayload, OnQueryPlanStartHookResult},
        on_subgraph_execute::{
            OnSubgraphExecuteStartHookPayload, OnSubgraphExecuteStartHookResult,
        },
        on_subgraph_http_request::{
            OnSubgraphHttpRequestHookPayload, OnSubgraphHttpRequestHookResult,
        },
        on_supergraph_load::{OnSupergraphLoadStartHookPayload, OnSupergraphLoadStartHookResult},
    },
    response::graphql_error::GraphQLError,
};
use serde::de::DeserializeOwned;
use sonic_rs::json;

pub struct StartHookResult<'exec, TStartPayload, TEndPayload, TResponse> {
    pub payload: TStartPayload,
    pub control_flow: StartControlFlow<'exec, TEndPayload, TResponse>,
}

pub enum StartControlFlow<'exec, TEndPayload, TResponse> {
    Proceed,
    EndWithResponse(TResponse),
    OnEnd(Box<dyn FnOnce(TEndPayload) -> EndHookResult<TEndPayload, TResponse> + Send + 'exec>),
}

// Override using methods (Like builder pattern)
// Async Drop
// Re-export Plugin related types from router crate (graphql_tools validation stuff, plugin stuff from internal crate)
// Move Plugin stuff from executor to internal

pub trait StartHookPayload<TEndPayload: EndHookPayload<TResponse>, TResponse>
where
    Self: Sized,
    TResponse: FromGraphQLErrorToResponse,
{
    /// Continue with the regular flow of the hook
    /// This is called in most cases when you don't short-circuit the hook with a response or an error.
    ///
    /// Example:
    /// ```
    /// async fn on_graphql_params<'exec>(
    ///     &'exec self,
    ///    payload: OnGraphQLParamsStartHookPayload<'exec>,
    /// ) -> OnGraphQLParamsStartHookResult<'exec> {
    ///    // manipulate payload if needed...
    ///    payload.proceed()
    /// }
    /// ```
    fn proceed<'exec>(self) -> StartHookResult<'exec, Self, TEndPayload, TResponse> {
        StartHookResult {
            payload: self,
            control_flow: StartControlFlow::Proceed,
        }
    }

    /// End the hook execution and return a response to the client immediately, skipping the rest of the execution flow.
    fn end_with_response<'exec, TResponseInput: Into<TResponse>>(
        self,
        output: TResponseInput,
    ) -> StartHookResult<'exec, Self, TEndPayload, TResponse> {
        StartHookResult {
            payload: self,
            control_flow: StartControlFlow::EndWithResponse(output.into()),
        }
    }

    /// End the hook execution with a GraphQL error,
    /// returning a response with the appropriate error format to the client immediately, skipping the rest of the execution flow.
    ///
    /// Example:
    /// ```
    /// fn on_http_request<'req>(
    ///     &'req self,
    ///     payload: OnHttpRequestHookPayload<'req>,
    /// ) -> OnHttpRequestHookResult<'req> {
    ///     if payload.router_http_request.headers().get("authorization").is_none() {
    ///         return payload.end_with_graphql_error(
    ///             GraphQLError::from_message_and_code("Unauthorized", "UNAUTHORIZED"),
    ///             StatusCode::UNAUTHORIZED,
    ///         );
    ///     }
    ///
    ///     payload.proceed()
    /// }
    /// ```
    fn end_with_graphql_error<'exec>(
        self,
        error: GraphQLError,
        status_code: http::StatusCode,
    ) -> StartHookResult<'exec, Self, TEndPayload, TResponse>
    where
        TResponse: FromGraphQLErrorToResponse,
    {
        self.end_with_response(TResponse::from_graphql_error_to_response(
            error,
            status_code,
        ))
    }

    /// End the hook execution with multiple GraphQL errors,
    /// returning a response with the appropriate error format to the client immediately, skipping the rest of the execution flow.
    fn end_with_graphql_errors<'exec, TErrors>(
        self,
        errors: TErrors,
        status_code: http::StatusCode,
    ) -> StartHookResult<'exec, Self, TEndPayload, TResponse>
    where
        TErrors: IntoIterator<Item = GraphQLError>,
        TResponse: FromGraphQLErrorsToResponse,
    {
        self.end_with_response(TResponse::from_graphql_errors_to_response(
            errors.into_iter().collect(),
            status_code,
        ))
    }

    /// Attach a callback to be executed at the end of the hook, allowing you to manipulate the end payload or response.
    /// This is useful when you want to execute some logic after the main execution of the hook
    ///
    /// Example:
    /// ```
    /// fn on_http_request<'req>(
    ///     &'req self,
    ///     payload: OnHttpRequestHookPayload<'req>,
    /// ) -> OnHttpRequestHookResult<'req> {
    ///     payload.on_end(|payload| {
    ///         payload.map_response(|mut response| {
    ///             response.response_mut().headers_mut().insert(
    ///                 "x-served-by",
    ///                 "hive-router".parse().unwrap(),
    ///             );
    ///             response
    ///         }).proceed()
    ///     })
    /// }
    /// ```
    fn on_end<'exec, F>(self, f: F) -> StartHookResult<'exec, Self, TEndPayload, TResponse>
    where
        F: FnOnce(TEndPayload) -> EndHookResult<TEndPayload, TResponse> + Send + 'exec,
    {
        StartHookResult {
            payload: self,
            control_flow: StartControlFlow::OnEnd(Box::new(f)),
        }
    }
}

pub struct EndHookResult<TEndPayload, TResponse> {
    pub payload: TEndPayload,
    pub control_flow: EndControlFlow<TResponse>,
}

pub enum EndControlFlow<TResponse> {
    Proceed,
    EndWithResponse(TResponse),
}

pub trait EndHookPayload<TResponse>
where
    Self: Sized,
    TResponse: FromGraphQLErrorToResponse,
{
    /// Continue with the regular flow of the hook
    /// This is called in most cases when you don't short-circuit the hook with a response or an error.
    fn proceed(self) -> EndHookResult<Self, TResponse> {
        EndHookResult {
            payload: self,
            control_flow: EndControlFlow::Proceed,
        }
    }

    /// End the hook execution and return a response to the client immediately, skipping the rest of the execution flow.
    fn end_with_response<TResponseInput: Into<TResponse>>(
        self,
        output: TResponseInput,
    ) -> EndHookResult<Self, TResponse> {
        EndHookResult {
            payload: self,
            control_flow: EndControlFlow::EndWithResponse(output.into()),
        }
    }

    /// End the hook execution with a GraphQL error,
    /// returning a response with the appropriate error format to the client immediately, skipping the rest of the execution flow.
    ///
    /// Example:
    /// ```
    /// use hive_router::{
    ///     plugins::hooks::on_http_request::{OnHttpRequestHookPayload, OnHttpRequestHookResult},
    /// };
    ///
    /// fn on_http_request<'req>(
    ///     &'req self,
    ///     payload: OnHttpRequestHookPayload<'req>,
    /// ) -> OnHttpRequestHookResult<'req> {
    ///     if payload.router_http_request.headers().get("authorization").is_none() {
    ///         return payload.end_with_graphql_error(
    ///             GraphQLError::from_message_and_code("Unauthorized", "UNAUTHORIZED"),
    ///             StatusCode::UNAUTHORIZED,
    ///         );
    ///     }
    ///
    ///     payload.proceed()
    /// }
    /// ```
    fn end_with_graphql_error(
        self,
        error: GraphQLError,
        status_code: http::StatusCode,
    ) -> EndHookResult<Self, TResponse> {
        self.end_with_response(TResponse::from_graphql_error_to_response(
            error,
            status_code,
        ))
    }

    /// End the hook execution with multiple GraphQL errors,
    /// returning a response with the appropriate error format to the client immediately, skipping the rest of the execution flow.
    fn end_with_graphql_errors<TErrors>(
        self,
        errors: TErrors,
        status_code: http::StatusCode,
    ) -> EndHookResult<Self, TResponse>
    where
        TErrors: IntoIterator<Item = GraphQLError>,
        TResponse: FromGraphQLErrorsToResponse,
    {
        self.end_with_response(TResponse::from_graphql_errors_to_response(
            errors.into_iter().collect(),
            status_code,
        ))
    }
}

pub trait FromGraphQLErrorToResponse {
    fn from_graphql_error_to_response(error: GraphQLError, status_code: http::StatusCode) -> Self;
}

pub trait FromGraphQLErrorsToResponse: FromGraphQLErrorToResponse {
    fn from_graphql_errors_to_response(
        errors: Vec<GraphQLError>,
        status_code: http::StatusCode,
    ) -> Self;
}

pub fn from_graphql_error_to_bytes(error: GraphQLError) -> Vec<u8> {
    from_graphql_errors_to_bytes(vec![error])
}

pub fn from_graphql_errors_to_bytes(errors: Vec<GraphQLError>) -> Vec<u8> {
    let body = json!({
        "errors": errors
    });
    sonic_rs::to_vec(&body).unwrap_or_default()
}

impl FromGraphQLErrorToResponse for ntex::http::Response {
    fn from_graphql_error_to_response(error: GraphQLError, status_code: http::StatusCode) -> Self {
        Self::from_graphql_errors_to_response(vec![error], status_code)
    }
}

impl FromGraphQLErrorsToResponse for ntex::http::Response {
    fn from_graphql_errors_to_response(
        errors: Vec<GraphQLError>,
        status_code: http::StatusCode,
    ) -> Self {
        let body = from_graphql_errors_to_bytes(errors);
        ntex::http::Response::build(ntex::http::StatusCode::OK)
            .content_type("application/json")
            .status(status_code)
            .body(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestStartPayload;
    struct TestEndPayload;

    struct TestResponse {
        errors: Vec<GraphQLError>,
        status_code: http::StatusCode,
    }

    impl FromGraphQLErrorToResponse for TestResponse {
        fn from_graphql_error_to_response(
            error: GraphQLError,
            status_code: http::StatusCode,
        ) -> Self {
            Self::from_graphql_errors_to_response(vec![error], status_code)
        }
    }

    impl FromGraphQLErrorsToResponse for TestResponse {
        fn from_graphql_errors_to_response(
            errors: Vec<GraphQLError>,
            status_code: http::StatusCode,
        ) -> Self {
            TestResponse {
                errors,
                status_code,
            }
        }
    }

    impl StartHookPayload<TestEndPayload, TestResponse> for TestStartPayload {}
    impl EndHookPayload<TestResponse> for TestEndPayload {}

    #[test]
    fn end_with_graphql_errors_returns_all_errors() {
        let result = TestStartPayload.end_with_graphql_errors(
            vec![
                GraphQLError::from_message_and_code("First violation", "FIRST_VIOLATION"),
                GraphQLError::from_message_and_code("Second violation", "SECOND_VIOLATION"),
            ],
            http::StatusCode::BAD_REQUEST,
        );

        let StartControlFlow::EndWithResponse(response) = result.control_flow else {
            panic!("expected hook to end with response");
        };

        assert_eq!(response.status_code, http::StatusCode::BAD_REQUEST);
        assert_eq!(response.errors.len(), 2);
        assert_eq!(response.errors[0].message, "First violation");
        assert_eq!(response.errors[1].message, "Second violation");
    }

    #[test]
    fn from_graphql_errors_to_bytes_serializes_all_errors() {
        let body = from_graphql_errors_to_bytes(vec![
            GraphQLError::from_message_and_code("First violation", "FIRST_VIOLATION"),
            GraphQLError::from_message_and_code("Second violation", "SECOND_VIOLATION"),
        ]);

        let body: sonic_rs::Value = sonic_rs::from_slice(&body).unwrap();

        assert_eq!(
            body,
            json!({
                "errors": [
                    {
                        "message": "First violation",
                        "extensions": {
                            "code": "FIRST_VIOLATION"
                        }
                    },
                    {
                        "message": "Second violation",
                        "extensions": {
                            "code": "SECOND_VIOLATION"
                        }
                    }
                ]
            })
        );
    }
}

#[async_trait::async_trait]
pub trait RouterPlugin: Send + Sync + 'static {
    fn plugin_name() -> &'static str;

    type Config: DeserializeOwned + Sync;

    fn on_plugin_init(payload: OnPluginInitPayload<Self>) -> OnPluginInitResult<Self>
    where
        Self: Sized;

    #[inline]
    fn on_http_request<'req>(
        &'req self,
        start_payload: OnHttpRequestHookPayload<'req>,
    ) -> OnHttpRequestHookResult<'req> {
        start_payload.proceed()
    }
    #[inline]
    async fn on_graphql_params<'exec>(
        &'exec self,
        start_payload: OnGraphQLParamsStartHookPayload<'exec>,
    ) -> OnGraphQLParamsStartHookResult<'exec> {
        start_payload.proceed()
    }
    #[inline]
    async fn on_graphql_parse<'exec>(
        &'exec self,
        start_payload: OnGraphQLParseStartHookPayload<'exec>,
    ) -> OnGraphQLParseHookResult<'exec> {
        start_payload.proceed()
    }
    #[inline]
    async fn on_graphql_validation<'exec>(
        &'exec self,
        start_payload: OnGraphQLValidationStartHookPayload<'exec>,
    ) -> OnGraphQLValidationStartHookResult<'exec> {
        start_payload.proceed()
    }
    #[inline]
    async fn on_query_plan<'exec>(
        &'exec self,
        start_payload: OnQueryPlanStartHookPayload<'exec>,
    ) -> OnQueryPlanStartHookResult<'exec> {
        start_payload.proceed()
    }
    #[inline]
    async fn on_execute<'exec>(
        &'exec self,
        start_payload: OnExecuteStartHookPayload<'exec>,
    ) -> OnExecuteStartHookResult<'exec> {
        start_payload.proceed()
    }
    #[inline]
    async fn on_subgraph_execute<'exec>(
        &'exec self,
        start_payload: OnSubgraphExecuteStartHookPayload<'exec>,
    ) -> OnSubgraphExecuteStartHookResult<'exec> {
        start_payload.proceed()
    }
    #[inline]
    async fn on_subgraph_http_request<'exec>(
        &'exec self,
        start_payload: OnSubgraphHttpRequestHookPayload<'exec>,
    ) -> OnSubgraphHttpRequestHookResult<'exec> {
        start_payload.proceed()
    }
    #[inline]
    fn on_supergraph_reload<'exec>(
        &'exec self,
        start_payload: OnSupergraphLoadStartHookPayload,
    ) -> OnSupergraphLoadStartHookResult<'exec> {
        start_payload.proceed()
    }
    #[inline]
    fn on_graphql_error<'req>(
        &'req self,
        payload: OnGraphQLErrorHookPayload<'req>,
    ) -> OnGraphQLErrorHookResult<'req> {
        payload.proceed()
    }
    #[inline]
    async fn on_shutdown<'exec>(&'exec self) {}
}

#[async_trait::async_trait]
pub trait DynRouterPlugin: Send + Sync + 'static {
    fn on_http_request<'req>(
        &'req self,
        start_payload: OnHttpRequestHookPayload<'req>,
    ) -> OnHttpRequestHookResult<'req>;
    async fn on_graphql_params<'exec>(
        &'exec self,
        start_payload: OnGraphQLParamsStartHookPayload<'exec>,
    ) -> OnGraphQLParamsStartHookResult<'exec>;
    async fn on_graphql_parse<'exec>(
        &'exec self,
        start_payload: OnGraphQLParseStartHookPayload<'exec>,
    ) -> OnGraphQLParseHookResult<'exec>;
    async fn on_graphql_validation<'exec>(
        &'exec self,
        start_payload: OnGraphQLValidationStartHookPayload<'exec>,
    ) -> OnGraphQLValidationStartHookResult<'exec>;
    async fn on_query_plan<'exec>(
        &'exec self,
        start_payload: OnQueryPlanStartHookPayload<'exec>,
    ) -> OnQueryPlanStartHookResult<'exec>;
    async fn on_execute<'exec>(
        &'exec self,
        start_payload: OnExecuteStartHookPayload<'exec>,
    ) -> OnExecuteStartHookResult<'exec>;
    async fn on_subgraph_execute<'exec>(
        &'exec self,
        start_payload: OnSubgraphExecuteStartHookPayload<'exec>,
    ) -> OnSubgraphExecuteStartHookResult<'exec>;
    async fn on_subgraph_http_request<'exec>(
        &'exec self,
        start_payload: OnSubgraphHttpRequestHookPayload<'exec>,
    ) -> OnSubgraphHttpRequestHookResult<'exec>;
    fn on_supergraph_reload<'exec>(
        &'exec self,
        start_payload: OnSupergraphLoadStartHookPayload,
    ) -> OnSupergraphLoadStartHookResult<'exec>;
    fn on_graphql_error<'req>(
        &'req self,
        payload: OnGraphQLErrorHookPayload<'req>,
    ) -> OnGraphQLErrorHookResult<'req>;
    async fn on_shutdown<'exec>(&'exec self);
}

#[async_trait::async_trait]
impl<P> DynRouterPlugin for P
where
    P: RouterPlugin,
{
    #[inline]
    fn on_http_request<'req>(
        &'req self,
        start_payload: OnHttpRequestHookPayload<'req>,
    ) -> OnHttpRequestHookResult<'req> {
        RouterPlugin::on_http_request(self, start_payload)
    }
    #[inline]
    async fn on_graphql_params<'exec>(
        &'exec self,
        start_payload: OnGraphQLParamsStartHookPayload<'exec>,
    ) -> OnGraphQLParamsStartHookResult<'exec> {
        RouterPlugin::on_graphql_params(self, start_payload).await
    }
    #[inline]
    async fn on_graphql_parse<'exec>(
        &'exec self,
        start_payload: OnGraphQLParseStartHookPayload<'exec>,
    ) -> OnGraphQLParseHookResult<'exec> {
        RouterPlugin::on_graphql_parse(self, start_payload).await
    }
    #[inline]
    async fn on_graphql_validation<'exec>(
        &'exec self,
        start_payload: OnGraphQLValidationStartHookPayload<'exec>,
    ) -> OnGraphQLValidationStartHookResult<'exec> {
        RouterPlugin::on_graphql_validation(self, start_payload).await
    }
    #[inline]
    async fn on_query_plan<'exec>(
        &'exec self,
        start_payload: OnQueryPlanStartHookPayload<'exec>,
    ) -> OnQueryPlanStartHookResult<'exec> {
        RouterPlugin::on_query_plan(self, start_payload).await
    }
    #[inline]
    async fn on_execute<'exec>(
        &'exec self,
        start_payload: OnExecuteStartHookPayload<'exec>,
    ) -> OnExecuteStartHookResult<'exec> {
        RouterPlugin::on_execute(self, start_payload).await
    }
    #[inline]
    async fn on_subgraph_execute<'exec>(
        &'exec self,
        start_payload: OnSubgraphExecuteStartHookPayload<'exec>,
    ) -> OnSubgraphExecuteStartHookResult<'exec> {
        RouterPlugin::on_subgraph_execute(self, start_payload).await
    }
    #[inline]
    async fn on_subgraph_http_request<'exec>(
        &'exec self,
        start_payload: OnSubgraphHttpRequestHookPayload<'exec>,
    ) -> OnSubgraphHttpRequestHookResult<'exec> {
        RouterPlugin::on_subgraph_http_request(self, start_payload).await
    }
    #[inline]
    fn on_supergraph_reload<'exec>(
        &'exec self,
        start_payload: OnSupergraphLoadStartHookPayload,
    ) -> OnSupergraphLoadStartHookResult<'exec> {
        RouterPlugin::on_supergraph_reload(self, start_payload)
    }
    #[inline]
    fn on_graphql_error<'req>(
        &'req self,
        payload: OnGraphQLErrorHookPayload<'req>,
    ) -> OnGraphQLErrorHookResult<'req> {
        RouterPlugin::on_graphql_error(self, payload)
    }
    #[inline]
    async fn on_shutdown<'exec>(&'exec self) {
        RouterPlugin::on_shutdown(self).await;
    }
}

pub type RouterPluginBoxed = Box<dyn DynRouterPlugin>;

pub enum CacheHint {
    Hit,
    Miss,
}

#[derive(Default)]
pub struct EarlyHTTPResponse {
    pub body: Vec<u8>,
    pub headers: http::HeaderMap,
    pub status_code: http::StatusCode,
}
