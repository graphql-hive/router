use serde::de::DeserializeOwned;

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
};

pub struct StartHookResult<'exec, TStartPayload, TEndPayload> {
    pub payload: TStartPayload,
    pub control_flow: StartControlFlow<'exec, TEndPayload>,
}

pub enum StartControlFlow<'exec, TEndPayload> {
    Continue,
    EndResponse(HttpResponse),
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
        output: HttpResponse,
    ) -> StartHookResult<'exec, Self, TEndPayload> {
        StartHookResult {
            payload: self,
            control_flow: StartControlFlow::EndResponse(output),
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
    EndResponse(HttpResponse),
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

    fn end_response(self, output: HttpResponse) -> EndHookResult<Self> {
        EndHookResult {
            payload: self,
            control_flow: EndControlFlow::EndResponse(output),
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
    async fn on_subgraph_execute<'exec>(
        &'exec self,
        start_payload: OnSubgraphExecuteStartHookPayload<'exec>,
    ) -> OnSubgraphExecuteStartHookResult<'exec> {
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
    async fn on_subgraph_execute<'exec>(
        &'exec self,
        start_payload: OnSubgraphExecuteStartHookPayload<'exec>,
    ) -> OnSubgraphExecuteStartHookResult<'exec> {
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
}

pub type RouterPluginBoxed = Box<dyn DynRouterPlugin>;
