use dashmap::DashMap;

use crate::{
    executors::common::HttpExecutionResponse,
    hooks::on_subgraph_execute::{OnSubgraphExecuteEndPayload, OnSubgraphExecuteStartPayload},
    plugin_trait::{EndPayload, HookResult, RouterPlugin, RouterPluginWithConfig, StartPayload},
};

impl RouterPluginWithConfig for SubgraphResponseCachePlugin {
    type Config = ();
    fn plugin_name() -> &'static str {
        "subgraph_response_cache_plugin"
    }
    fn new(_config: ()) -> Self {
        SubgraphResponseCachePlugin {
            cache: DashMap::new(),
        }
    }
}

pub struct SubgraphResponseCachePlugin {
    cache: DashMap<String, HttpExecutionResponse>,
}

#[async_trait::async_trait]
impl RouterPlugin for SubgraphResponseCachePlugin {
    async fn on_subgraph_execute<'exec>(
        &'exec self,
        mut payload: OnSubgraphExecuteStartPayload<'exec>,
    ) -> HookResult<'exec, OnSubgraphExecuteStartPayload<'exec>, OnSubgraphExecuteEndPayload> {
        let key = format!(
            "subgraph_response_cache:{}:{:?}",
            payload.execution_request.query, payload.execution_request.variables
        );
        if let Some(cached_response) = self.cache.get(&key) {
            // Here payload.response is Option
            // So it is bypassing the actual subgraph request
            payload.execution_result = Some(cached_response.clone());
            return payload.cont();
        }
        payload.on_end(move |payload: OnSubgraphExecuteEndPayload| {
            // Here payload.response is not Option
            self.cache.insert(key, payload.execution_result.clone());
            payload.cont()
        })
    }
}
