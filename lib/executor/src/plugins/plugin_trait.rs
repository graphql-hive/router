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
    fn proceed<'exec>(self) -> StartHookResult<'exec, Self, TEndPayload, TResponse> {
        StartHookResult {
            payload: self,
            control_flow: StartControlFlow::Proceed,
        }
    }

    fn end_with_response<'exec>(
        self,
        output: TResponse,
    ) -> StartHookResult<'exec, Self, TEndPayload, TResponse> {
        StartHookResult {
            payload: self,
            control_flow: StartControlFlow::EndWithResponse(output),
        }
    }

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
    fn proceed(self) -> EndHookResult<Self, TResponse> {
        EndHookResult {
            payload: self,
            control_flow: EndControlFlow::Proceed,
        }
    }

    fn end_with_response(self, output: TResponse) -> EndHookResult<Self, TResponse> {
        EndHookResult {
            payload: self,
            control_flow: EndControlFlow::EndWithResponse(output),
        }
    }

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
}

pub trait FromGraphQLErrorToResponse {
    fn from_graphql_error_to_response(error: GraphQLError, status_code: http::StatusCode) -> Self;
}

pub fn from_graphql_error_to_bytes(error: GraphQLError) -> Vec<u8> {
    let body = json!({
        "errors": [error]
    });
    sonic_rs::to_vec(&body).unwrap_or_default()
}

impl FromGraphQLErrorToResponse for ntex::http::Response {
    fn from_graphql_error_to_response(error: GraphQLError, status_code: http::StatusCode) -> Self {
        let body = from_graphql_error_to_bytes(error);
        ntex::http::Response::build(ntex::http::StatusCode::OK)
            .content_type("application/json")
            .status(status_code)
            .body(body)
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
    fn on_graphql_error(&self, payload: OnGraphQLErrorHookPayload) -> OnGraphQLErrorHookResult {
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
    fn on_graphql_error(&self, payload: OnGraphQLErrorHookPayload) -> OnGraphQLErrorHookResult;
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
    fn on_graphql_error(&self, payload: OnGraphQLErrorHookPayload) -> OnGraphQLErrorHookResult {
        RouterPlugin::on_graphql_error(self, payload)
    }
    #[inline]
    async fn on_shutdown<'exec>(&'exec self) {
        RouterPlugin::on_shutdown(self).await;
    }
}

pub type RouterPluginBoxed = Box<dyn DynRouterPlugin>;
