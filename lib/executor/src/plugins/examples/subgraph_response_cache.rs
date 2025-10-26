use dashmap::DashMap;

use crate::{executors::dedupe::SharedResponse, hooks::on_subgraph_http_request::{OnSubgraphHttpRequestPayload, OnSubgraphHttpResponsePayload}, plugin_trait::{ControlFlow, RouterPlugin}};

pub struct SubgraphResponseCachePlugin {
    cache: DashMap<String, SharedResponse>,
}

impl RouterPlugin for SubgraphResponseCachePlugin {
    fn on_subgraph_http_request<'exec>(
            &'static self, 
            payload: OnSubgraphHttpRequestPayload<'exec>,
        ) -> ControlFlow<'exec, OnSubgraphHttpResponsePayload<'exec>> {
        let key = format!(
            "subgraph_response_cache:{}:{:?}",
            payload.execution_request.query, payload.execution_request.variables
        );
        if let Some(cached_response) = self.cache.get(&key) {
            // Here payload.response is Option
            // So it is bypassing the actual subgraph request
            *payload.response = Some(cached_response.clone());
            return ControlFlow::Continue;
        }
        ControlFlow::OnEnd(Box::new(move |payload: OnSubgraphHttpResponsePayload| {
            // Here payload.response is not Option
            self.cache.insert(key, payload.response.clone());
            ControlFlow::Continue
        }))
    }
}