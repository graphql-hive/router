use serde::de::DeserializeOwned;

use crate::execution::plan::PlanExecutionOutput;
use crate::hooks::on_execute::{OnExecuteEndPayload, OnExecuteStartPayload};
use crate::hooks::on_graphql_params::{OnGraphQLParamsEndPayload, OnGraphQLParamsStartPayload};
use crate::hooks::on_graphql_parse::{OnGraphQLParseEndPayload, OnGraphQLParseStartPayload};
use crate::hooks::on_graphql_validation::{
    OnGraphQLValidationEndPayload, OnGraphQLValidationStartPayload,
};
use crate::hooks::on_http_request::{OnHttpRequestPayload, OnHttpResponsePayload};
use crate::hooks::on_query_plan::{OnQueryPlanEndPayload, OnQueryPlanStartPayload};
use crate::hooks::on_subgraph_execute::{
    OnSubgraphExecuteEndPayload, OnSubgraphExecuteStartPayload,
};
use crate::hooks::on_subgraph_http_request::{
    OnSubgraphHttpRequestPayload, OnSubgraphHttpResponsePayload,
};
use crate::hooks::on_supergraph_load::{OnSupergraphLoadEndPayload, OnSupergraphLoadStartPayload};

pub struct HookResult<'exec, TStartPayload, TEndPayload> {
    pub payload: TStartPayload,
    pub control_flow: ControlFlowResult<'exec, TEndPayload>,
}

pub enum ControlFlowResult<'exec, TEndPayload> {
    Continue,
    EndResponse(PlanExecutionOutput),
    OnEnd(Box<dyn FnOnce(TEndPayload) -> HookResult<'exec, TEndPayload, ()> + Send + 'exec>),
}

pub trait StartPayload<TEndPayload: EndPayload>
where
    Self: Sized,
{
    fn cont<'exec>(self) -> HookResult<'exec, Self, TEndPayload> {
        HookResult {
            payload: self,
            control_flow: ControlFlowResult::Continue,
        }
    }

    fn end_response<'exec>(
        self,
        output: PlanExecutionOutput,
    ) -> HookResult<'exec, Self, TEndPayload> {
        HookResult {
            payload: self,
            control_flow: ControlFlowResult::EndResponse(output),
        }
    }

    fn on_end<'exec, F>(self, f: F) -> HookResult<'exec, Self, TEndPayload>
    where
        F: FnOnce(TEndPayload) -> HookResult<'exec, TEndPayload, ()> + Send + 'exec,
    {
        HookResult {
            payload: self,
            control_flow: ControlFlowResult::OnEnd(Box::new(f)),
        }
    }
}

pub trait EndPayload
where
    Self: Sized,
{
    fn cont<'exec>(self) -> HookResult<'exec, Self, ()> {
        HookResult {
            payload: self,
            control_flow: ControlFlowResult::Continue,
        }
    }

    fn end_response<'exec>(self, output: PlanExecutionOutput) -> HookResult<'exec, Self, ()> {
        HookResult {
            payload: self,
            control_flow: ControlFlowResult::EndResponse(output),
        }
    }
}

pub trait RouterPluginWithConfig
where
    Self: Sized,
    Self: RouterPlugin,
{
    fn plugin_name() -> &'static str;
    type Config: Send + Sync + DeserializeOwned;
    fn from_config(config: Self::Config) -> Option<Self>;
    fn from_config_value(value: serde_json::Value) -> serde_json::Result<Option<Box<Self>>>
    where
        Self: Sized,
    {
        let config: Self::Config = serde_json::from_value(value)?;
        let plugin = Self::from_config(config);
        match plugin {
            None => Ok(None),
            Some(plugin) => Ok(Some(Box::new(plugin))),
        }
    }
}

#[async_trait::async_trait]
pub trait RouterPlugin {
    fn on_http_request<'req>(
        &'req self,
        start_payload: OnHttpRequestPayload<'req>,
    ) -> HookResult<'req, OnHttpRequestPayload<'req>, OnHttpResponsePayload<'req>> {
        start_payload.cont()
    }
    async fn on_graphql_params<'exec>(
        &'exec self,
        start_payload: OnGraphQLParamsStartPayload<'exec>,
    ) -> HookResult<'exec, OnGraphQLParamsStartPayload<'exec>, OnGraphQLParamsEndPayload> {
        start_payload.cont()
    }
    async fn on_graphql_parse<'exec>(
        &'exec self,
        start_payload: OnGraphQLParseStartPayload<'exec>,
    ) -> HookResult<'exec, OnGraphQLParseStartPayload<'exec>, OnGraphQLParseEndPayload> {
        start_payload.cont()
    }
    async fn on_graphql_validation<'exec>(
        &'exec self,
        start_payload: OnGraphQLValidationStartPayload<'exec>,
    ) -> HookResult<'exec, OnGraphQLValidationStartPayload<'exec>, OnGraphQLValidationEndPayload>
    {
        start_payload.cont()
    }
    async fn on_query_plan<'exec>(
        &'exec self,
        start_payload: OnQueryPlanStartPayload<'exec>,
    ) -> HookResult<'exec, OnQueryPlanStartPayload<'exec>, OnQueryPlanEndPayload> {
        start_payload.cont()
    }
    async fn on_execute<'exec>(
        &'exec self,
        start_payload: OnExecuteStartPayload<'exec>,
    ) -> HookResult<'exec, OnExecuteStartPayload<'exec>, OnExecuteEndPayload<'exec>> {
        start_payload.cont()
    }
    async fn on_subgraph_execute<'exec>(
        &'exec self,
        start_payload: OnSubgraphExecuteStartPayload<'exec>,
    ) -> HookResult<'exec, OnSubgraphExecuteStartPayload<'exec>, OnSubgraphExecuteEndPayload> {
        start_payload.cont()
    }
    async fn on_subgraph_http_request<'exec>(
        &'exec self,
        start_payload: OnSubgraphHttpRequestPayload<'exec>,
    ) -> HookResult<'exec, OnSubgraphHttpRequestPayload<'exec>, OnSubgraphHttpResponsePayload> {
        start_payload.cont()
    }
    fn on_supergraph_reload<'exec>(
        &'exec self,
        start_payload: OnSupergraphLoadStartPayload,
    ) -> HookResult<'exec, OnSupergraphLoadStartPayload, OnSupergraphLoadEndPayload> {
        start_payload.cont()
    }
}
