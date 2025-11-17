use crate::execution::plan::PlanExecutionOutput;
use crate::hooks::on_deserialization::{OnDeserializationEndPayload, OnDeserializationStartPayload};
use crate::hooks::on_execute::{OnExecuteEndPayload, OnExecuteStartPayload};
use crate::hooks::on_graphql_parse::{OnGraphQLParseEndPayload, OnGraphQLParseStartPayload};
use crate::hooks::on_graphql_validation::{OnGraphQLValidationEndPayload, OnGraphQLValidationStartPayload};
use crate::hooks::on_http_request::{OnHttpRequestPayload, OnHttpResponse};
use crate::hooks::on_query_plan::{OnQueryPlanEndPayload, OnQueryPlanStartPayload};
use crate::hooks::on_schema_reload::OnSchemaReloadPayload;
use crate::hooks::on_subgraph_http_request::{OnSubgraphHttpRequestPayload, OnSubgraphHttpResponsePayload};

pub struct HookResult<'exec, TStartPayload, TEndPayload> {
    pub start_payload: TStartPayload,
    pub control_flow: ControlFlowResult<'exec, TEndPayload>,
}

pub enum ControlFlowResult<'exec, TEndPayload> {
    Continue,
    EndResponse(PlanExecutionOutput),
    OnEnd(Box<dyn FnOnce(TEndPayload) -> HookResult<'exec, TEndPayload, ()> + 'exec>),
}

pub trait StartPayload<TEndPayload: EndPayload>
    where Self: Sized
    {

    fn cont<'exec>(self) -> HookResult<'exec, Self, TEndPayload> {
        HookResult {
            start_payload: self,
            control_flow: ControlFlowResult::Continue,
        }
    }

    fn end_response<'exec>(self, output: PlanExecutionOutput) -> HookResult<'exec, Self, TEndPayload> {
        HookResult {
            start_payload: self,
            control_flow: ControlFlowResult::EndResponse(output),
        }
    }

    fn on_end<'exec, F>(self, f: F) -> HookResult<'exec, Self, TEndPayload>
        where F: FnOnce(TEndPayload) -> HookResult<'exec, TEndPayload, ()> + 'exec,
    {
        HookResult {
            start_payload: self,
            control_flow: ControlFlowResult::OnEnd(Box::new(f)),
        }
    }
}

pub trait EndPayload
    where Self: Sized
    {
        fn cont<'exec>(self) -> HookResult<'exec, Self, ()> {
            HookResult {
                start_payload: self,
                control_flow: ControlFlowResult::Continue,
            }
        }

        fn end_response<'exec>(self, output: PlanExecutionOutput) -> HookResult<'exec, Self, ()> {
            HookResult {
                start_payload: self,
                control_flow: ControlFlowResult::EndResponse(output),
            }
        }
}

// Add sync send etc
pub trait RouterPlugin {
    fn on_http_request<'exec>(
        &self, 
        start_payload: OnHttpRequestPayload<'exec>,
    ) -> HookResult<'exec, OnHttpRequestPayload<'exec>, OnHttpResponse<'exec>> {
        start_payload.cont()
    }
    fn on_deserialization<'exec>(
        &'exec self, 
        start_payload: OnDeserializationStartPayload<'exec>,
    ) -> HookResult<'exec, OnDeserializationStartPayload<'exec>, OnDeserializationEndPayload<'exec>> {
        start_payload.cont()
    }
    fn on_graphql_parse<'exec>(
        &self, 
        start_payload: OnGraphQLParseStartPayload<'exec>,
    ) -> HookResult<'exec, OnGraphQLParseStartPayload<'exec>, OnGraphQLParseEndPayload<'exec>> {
        start_payload.cont()
    }
    fn on_graphql_validation<'exec>(
        &self, 
        start_payload: OnGraphQLValidationStartPayload<'exec>,
    ) -> HookResult<'exec, OnGraphQLValidationStartPayload<'exec>, OnGraphQLValidationEndPayload<'exec>> {
        start_payload.cont()
    }
    fn on_query_plan<'exec>(
        &self, 
        start_payload: OnQueryPlanStartPayload<'exec>,
    ) ->  HookResult<'exec, OnQueryPlanStartPayload<'exec>, OnQueryPlanEndPayload<'exec>> {
        start_payload.cont()
    }
    fn on_execute<'exec>(
        &'exec self, 
        start_payload: OnExecuteStartPayload<'exec>,
    ) ->  HookResult<'exec, OnExecuteStartPayload<'exec>, OnExecuteEndPayload<'exec>> {
        start_payload.cont()
    }
    fn on_subgraph_http_request<'exec>(
        &'static self, 
        start_payload: OnSubgraphHttpRequestPayload<'exec>,
    ) -> HookResult<'exec, OnSubgraphHttpRequestPayload<'exec>, OnSubgraphHttpResponsePayload<'exec>> {
        start_payload.cont()
    }
    fn on_schema_reload<'a>(&'a self, _start_payload: OnSchemaReloadPayload) {}
}