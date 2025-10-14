use dashmap::DashMap;

use crate::{hooks::on_subgraph_execute::{OnSubgraphExecuteEndPayload, OnSubgraphExecuteStartPayload, SubgraphExecutorResponse, SubgraphResponse}, plugin_trait::{ControlFlow, RouterPlugin}};

struct SubgraphResponseCachePlugin {
    cache: DashMap<String, SubgraphResponse<'static>>,
}

impl RouterPlugin for SubgraphResponseCachePlugin {
    fn on_subgraph_execute<'exec>(
            &self, 
            payload: OnSubgraphExecuteStartPayload<'exec>,
        ) -> ControlFlow<'exec, OnSubgraphExecuteEndPayload<'exec>> {
        let key = format!(
            "subgraph_response_cache:{}:{}:{:?}",
            payload.subgraph_name, payload.execution_request.operation_name.unwrap_or(""), payload.execution_request.variables
        );
        if let Some(cached_response) = self.cache.get(&key) {
            *payload.response = Some(SubgraphExecutorResponse::RawResponse(cached_response));
            // Return early with the cached response
            return ControlFlow::Continue;
        } else {
            ControlFlow::OnEnd(Box::new(move |payload: OnSubgraphExecuteEndPayload| {
                let cacheable = payload.response.errors.is_none_or(|errors| errors.is_empty());
                if cacheable {
                    self.cache.insert(key, *payload.response);
                }
                ControlFlow::Continue
            }))
        }
    }
}