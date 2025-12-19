use dashmap::DashMap;
use serde::Deserialize;

use hive_router_plan_executor::{
    executors::http::HttpResponse,
    hooks::on_subgraph_execute::{
        OnSubgraphExecuteEndHookPayload, OnSubgraphExecuteStartHookPayload,
        OnSubgraphExecuteStartHookResult,
    },
    plugin_trait::{EndHookPayload, RouterPlugin, StartHookPayload},
};

#[derive(Deserialize)]
pub struct SubgraphResponseCachePluginConfig {
    enabled: bool,
}

pub struct SubgraphResponseCachePlugin {
    cache: DashMap<String, HttpResponse>,
}

#[async_trait::async_trait]
impl RouterPlugin for SubgraphResponseCachePlugin {
    type Config = SubgraphResponseCachePluginConfig;
    fn plugin_name() -> &'static str {
        "subgraph_response_cache"
    }
    fn from_config(config: SubgraphResponseCachePluginConfig) -> Option<Self> {
        if config.enabled {
            Some(SubgraphResponseCachePlugin {
                cache: DashMap::new(),
            })
        } else {
            None
        }
    }
    async fn on_subgraph_execute<'exec>(
        &'exec self,
        mut payload: OnSubgraphExecuteStartHookPayload<'exec>,
    ) -> OnSubgraphExecuteStartHookResult<'exec> {
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
        payload.on_end(move |payload: OnSubgraphExecuteEndHookPayload| {
            // Here payload.response is not Option
            self.cache.insert(key, payload.execution_result.clone());
            payload.cont()
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::testkit::{
        init_graphql_request, init_router_from_config_inline, wait_for_readiness, SubgraphsServer,
    };
    use hive_router::PluginRegistry;
    use ntex::web::test;

    // Tests on_subgraph_execute's override behavior
    #[ntex::test]
    async fn caches_subgraph_responses() {
        let subgraphs = SubgraphsServer::start().await;
        let app = init_router_from_config_inline(
            r#"
            plugins:
                subgraph_response_cache:
                    enabled: true
            "#,
            Some(PluginRegistry::new().register::<super::SubgraphResponseCachePlugin>()),
        )
        .await
        .expect("failed to start router");
        wait_for_readiness(&app.app).await;
        let req = init_graphql_request("{ users { id } }", None);
        let resp = test::call_service(&app.app, req.to_request()).await;
        assert!(resp.status().is_success());
        let req = init_graphql_request("{ users { id } }", None);
        let resp2 = test::call_service(&app.app, req.to_request()).await;
        assert!(resp2.status().is_success());
        let subgraph_requests = subgraphs
            .get_subgraph_requests_log("accounts")
            .await
            .expect("failed to get subgraph requests log");
        assert_eq!(subgraph_requests.len(), 1);
    }
}
