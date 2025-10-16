use ntex::web::HttpResponse;

use crate::hooks::on_execute::OnExecutePayload;
use crate::hooks::on_schema_reload::OnSchemaReloadPayload;
use crate::hooks::on_subgraph_http_request::{OnSubgraphHttpRequestPayload, OnSubgraphHttpResponsePayload};

pub enum ControlFlow<'exec, TPayload> {
    Continue,
    Break(HttpResponse),
    OnEnd(Box<dyn FnOnce(TPayload) -> ControlFlow<'exec, ()> + 'exec>),
}

pub trait RouterPlugin {
    fn on_execute<'exec>(
        &self, 
        _payload: OnExecutePayload<'exec>,
    ) -> ControlFlow<'exec, OnExecutePayload<'exec>> {
        ControlFlow::Continue
    }
    fn on_subgraph_http_request<'exec>(
        &'static self, 
        _payload: OnSubgraphHttpRequestPayload<'exec>,
    ) -> ControlFlow<'exec, OnSubgraphHttpResponsePayload<'exec>> {
        ControlFlow::Continue
    }
    fn on_schema_reload(&self, _payload: OnSchemaReloadPayload) {}
}