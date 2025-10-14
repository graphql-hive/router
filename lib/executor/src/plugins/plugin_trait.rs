use ntex::web::HttpResponse;

use crate::hooks::on_execute::OnExecutePayload;
use crate::hooks::on_schema_reload::OnSchemaReloadPayload;
use crate::hooks::on_subgraph_execute::OnSubgraphExecuteEndPayload;
use crate::hooks::on_subgraph_execute::OnSubgraphExecuteStartPayload;

pub enum ControlFlow<'a, TPayload> {
    Continue,
    Break(HttpResponse),
    OnEnd(Box<dyn FnOnce(TPayload) -> ControlFlow<'a, ()> + Send + 'a>),
}

pub trait RouterPlugin {
    fn on_execute<'exec>(
        &self, 
        _payload: OnExecutePayload<'exec>,
    ) -> ControlFlow<'exec, OnExecutePayload<'exec>> {
        ControlFlow::Continue
    }
    fn on_subgraph_execute<'exec>(
        &self, 
        _payload: OnSubgraphExecuteStartPayload<'exec>,
    ) -> ControlFlow<'exec, OnSubgraphExecuteEndPayload<'exec>> {
        ControlFlow::Continue
    }
    fn on_schema_reload(&self, _payload: OnSchemaReloadPayload) {}
}