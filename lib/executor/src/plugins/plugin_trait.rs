use std::sync::Arc;

use http::StatusCode;
use serde::{de::DeserializeOwned, Serialize};
use sonic_rs::json;

use crate::{
    executors::http::HttpResponse,
    hooks::{
        on_execute::{OnExecuteStartHookPayload, OnExecuteStartHookResult},
        on_graphql_params::{OnGraphQLParamsStartHookPayload, OnGraphQLParamsStartHookResult},
        on_graphql_parse::{OnGraphQLParseHookResult, OnGraphQLParseStartHookPayload},
        on_graphql_validation::{
            OnGraphQLValidationStartHookPayload, OnGraphQLValidationStartHookResult,
        },
        on_http_request::{OnHttpRequestHookPayload, OnHttpRequestHookResult},
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

pub struct StartHookResult<'exec, TStartPayload, TEndPayload> {
    pub payload: TStartPayload,
    pub control_flow: StartControlFlow<'exec, TEndPayload>,
}

pub enum StartControlFlow<'exec, TEndPayload> {
    Continue,
    EndResponse(Arc<HttpResponse>),
    OnEnd(Box<dyn FnOnce(TEndPayload) -> EndHookResult<TEndPayload> + Send + 'exec>),
}

// Override using methods (Like builder pattern)
// Async Drop
// Re-export Plugin related types from router crate (graphql_tools validation stuff, plugin stuff from internal crate)
// Move Plugin stuff from executor to internal

pub trait StartHookPayload<TEndPayload: EndHookPayload>
where
    Self: Sized,
{
    fn cont<'exec>(self) -> StartHookResult<'exec, Self, TEndPayload> {
        StartHookResult {
            payload: self,
            control_flow: StartControlFlow::Continue,
        }
    }

    fn end_response<'exec>(
        self,
        output: Arc<HttpResponse>,
    ) -> StartHookResult<'exec, Self, TEndPayload> {
        StartHookResult {
            payload: self,
            control_flow: StartControlFlow::EndResponse(output),
        }
    }

    fn end_response_body<'exec, T: Serialize>(
        self,
        body: T,
    ) -> StartHookResult<'exec, Self, TEndPayload> {
        let http_response = HttpResponse {
            status: StatusCode::BAD_REQUEST,
            headers: Default::default(),
            body: Arc::new(sonic_rs::to_vec(&body).unwrap().into()),
        };
        StartHookResult {
            payload: self,
            control_flow: StartControlFlow::EndResponse(http_response.into()),
        }
    }

    fn end_graphql_error<'exec>(
        self,
        error: GraphQLError,
        status: StatusCode,
    ) -> StartHookResult<'exec, Self, TEndPayload> {
        let body = json!({
            "errors": [error]
        });
        let http_response = HttpResponse {
            status,
            headers: Default::default(),
            body: Arc::new(sonic_rs::to_vec(&body).unwrap().into()),
        };
        StartHookResult {
            payload: self,
            control_flow: StartControlFlow::EndResponse(http_response.into()),
        }
    }

    fn on_end<'exec, F>(self, f: F) -> StartHookResult<'exec, Self, TEndPayload>
    where
        F: FnOnce(TEndPayload) -> EndHookResult<TEndPayload> + Send + 'exec,
    {
        StartHookResult {
            payload: self,
            control_flow: StartControlFlow::OnEnd(Box::new(f)),
        }
    }
}

pub struct EndHookResult<TEndPayload> {
    pub payload: TEndPayload,
    pub control_flow: EndControlFlow,
}

pub enum EndControlFlow {
    Continue,
    EndResponse(Arc<HttpResponse>),
}

pub trait EndHookPayload
where
    Self: Sized,
{
    fn cont(self) -> EndHookResult<Self> {
        EndHookResult {
            payload: self,
            control_flow: EndControlFlow::Continue,
        }
    }

    fn end_response(self, output: Arc<HttpResponse>) -> EndHookResult<Self> {
        EndHookResult {
            payload: self,
            control_flow: EndControlFlow::EndResponse(output),
        }
    }

    fn end_response_body<T: Serialize>(self, body: T) -> EndHookResult<Self> {
        let http_response = HttpResponse {
            status: StatusCode::OK,
            headers: Default::default(),
            body: Arc::new(sonic_rs::to_vec(&body).unwrap().into()),
        };
        EndHookResult {
            payload: self,
            control_flow: EndControlFlow::EndResponse(http_response.into()),
        }
    }

    fn end_graphql_error(self, error: GraphQLError) -> EndHookResult<Self> {
        let body = json!({
            "errors": [error]
        });
        let http_response = HttpResponse {
            status: StatusCode::BAD_REQUEST,
            headers: Default::default(),
            body: Arc::new(sonic_rs::to_vec(&body).unwrap().into()),
        };
        EndHookResult {
            payload: self,
            control_flow: EndControlFlow::EndResponse(http_response.into()),
        }
    }
}

#[async_trait::async_trait]
pub trait RouterPlugin: Send + Sync + 'static {
    fn plugin_name() -> &'static str;

    type Config: DeserializeOwned + Sync;

    fn from_config(config: Self::Config) -> Option<Self>
    where
        Self: Sized;

    fn as_dyn(&self) -> &dyn DynRouterPlugin
    where
        Self: Sized,
        Self: DynRouterPlugin,
    {
        self as &dyn DynRouterPlugin
    }

    fn on_http_request<'req>(
        &'req self,
        start_payload: OnHttpRequestHookPayload<'req>,
    ) -> OnHttpRequestHookResult<'req> {
        start_payload.cont()
    }
    async fn on_graphql_params<'exec>(
        &'exec self,
        start_payload: OnGraphQLParamsStartHookPayload<'exec>,
    ) -> OnGraphQLParamsStartHookResult<'exec> {
        start_payload.cont()
    }
    async fn on_graphql_parse<'exec>(
        &'exec self,
        start_payload: OnGraphQLParseStartHookPayload<'exec>,
    ) -> OnGraphQLParseHookResult<'exec> {
        start_payload.cont()
    }
    async fn on_graphql_validation<'exec>(
        &'exec self,
        start_payload: OnGraphQLValidationStartHookPayload<'exec>,
    ) -> OnGraphQLValidationStartHookResult<'exec> {
        start_payload.cont()
    }
    async fn on_query_plan<'exec>(
        &'exec self,
        start_payload: OnQueryPlanStartHookPayload<'exec>,
    ) -> OnQueryPlanStartHookResult<'exec> {
        start_payload.cont()
    }
    async fn on_execute<'exec>(
        &'exec self,
        start_payload: OnExecuteStartHookPayload<'exec>,
    ) -> OnExecuteStartHookResult<'exec> {
        start_payload.cont()
    }
    async fn on_subgraph_execute<'exec, 'req>(
        &'exec self,
        start_payload: OnSubgraphExecuteStartHookPayload<'exec, 'req>,
    ) -> OnSubgraphExecuteStartHookResult<'exec, 'req> {
        start_payload.cont()
    }
    async fn on_subgraph_http_request<'exec>(
        &'exec self,
        start_payload: OnSubgraphHttpRequestHookPayload<'exec>,
    ) -> OnSubgraphHttpRequestHookResult<'exec> {
        start_payload.cont()
    }
    fn on_supergraph_reload<'exec>(
        &'exec self,
        start_payload: OnSupergraphLoadStartHookPayload,
    ) -> OnSupergraphLoadStartHookResult<'exec> {
        start_payload.cont()
    }
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
    async fn on_subgraph_execute<'exec, 'req>(
        &'exec self,
        start_payload: OnSubgraphExecuteStartHookPayload<'exec, 'req>,
    ) -> OnSubgraphExecuteStartHookResult<'exec, 'req>;
    async fn on_subgraph_http_request<'exec>(
        &'exec self,
        start_payload: OnSubgraphHttpRequestHookPayload<'exec>,
    ) -> OnSubgraphHttpRequestHookResult<'exec>;
    fn on_supergraph_reload<'exec>(
        &'exec self,
        start_payload: OnSupergraphLoadStartHookPayload,
    ) -> OnSupergraphLoadStartHookResult<'exec>;
    async fn on_shutdown<'exec>(&'exec self);
}

#[async_trait::async_trait]
impl<P> DynRouterPlugin for P
where
    P: RouterPlugin,
{
    fn on_http_request<'req>(
        &'req self,
        start_payload: OnHttpRequestHookPayload<'req>,
    ) -> OnHttpRequestHookResult<'req> {
        RouterPlugin::on_http_request(self, start_payload)
    }
    async fn on_graphql_params<'exec>(
        &'exec self,
        start_payload: OnGraphQLParamsStartHookPayload<'exec>,
    ) -> OnGraphQLParamsStartHookResult<'exec> {
        RouterPlugin::on_graphql_params(self, start_payload).await
    }
    async fn on_graphql_parse<'exec>(
        &'exec self,
        start_payload: OnGraphQLParseStartHookPayload<'exec>,
    ) -> OnGraphQLParseHookResult<'exec> {
        RouterPlugin::on_graphql_parse(self, start_payload).await
    }
    async fn on_graphql_validation<'exec>(
        &'exec self,
        start_payload: OnGraphQLValidationStartHookPayload<'exec>,
    ) -> OnGraphQLValidationStartHookResult<'exec> {
        RouterPlugin::on_graphql_validation(self, start_payload).await
    }
    async fn on_query_plan<'exec>(
        &'exec self,
        start_payload: OnQueryPlanStartHookPayload<'exec>,
    ) -> OnQueryPlanStartHookResult<'exec> {
        RouterPlugin::on_query_plan(self, start_payload).await
    }
    async fn on_execute<'exec>(
        &'exec self,
        start_payload: OnExecuteStartHookPayload<'exec>,
    ) -> OnExecuteStartHookResult<'exec> {
        RouterPlugin::on_execute(self, start_payload).await
    }
    async fn on_subgraph_execute<'exec, 'req>(
        &'exec self,
        start_payload: OnSubgraphExecuteStartHookPayload<'exec, 'req>,
    ) -> OnSubgraphExecuteStartHookResult<'exec, 'req> {
        RouterPlugin::on_subgraph_execute(self, start_payload).await
    }
    async fn on_subgraph_http_request<'exec>(
        &'exec self,
        start_payload: OnSubgraphHttpRequestHookPayload<'exec>,
    ) -> OnSubgraphHttpRequestHookResult<'exec> {
        RouterPlugin::on_subgraph_http_request(self, start_payload).await
    }
    fn on_supergraph_reload<'exec>(
        &'exec self,
        start_payload: OnSupergraphLoadStartHookPayload,
    ) -> OnSupergraphLoadStartHookResult<'exec> {
        RouterPlugin::on_supergraph_reload(self, start_payload)
    }
    async fn on_shutdown<'exec>(&'exec self) {
        RouterPlugin::on_shutdown(self).await;
    }
}

pub type RouterPluginBoxed = Box<dyn DynRouterPlugin>;
