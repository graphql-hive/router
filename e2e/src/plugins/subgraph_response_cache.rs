use dashmap::DashMap;
use hive_router::BoxError;
use serde::Deserialize;

use hive_router_plan_executor::{
    executors::http::HttpResponse,
    hooks::on_subgraph_http_request::{
        OnSubgraphHttpRequestHookPayload, OnSubgraphHttpRequestHookResult,
        OnSubgraphHttpResponseHookPayload,
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
    fn from_config(config: SubgraphResponseCachePluginConfig) -> Result<Option<Self>, BoxError> {
        if config.enabled {
            Ok(Some(SubgraphResponseCachePlugin {
                cache: DashMap::new(),
            }))
        } else {
            Ok(None)
        }
    }
    async fn on_subgraph_http_request<'exec>(
        &'exec self,
        payload: OnSubgraphHttpRequestHookPayload<'exec>,
    ) -> OnSubgraphHttpRequestHookResult<'exec> {
        let key = format!(
            "subgraph_response_cache:{}:{:?}",
            payload.execution_request.query, payload.execution_request.variables
        );
        if let Some(cached_response) = self.cache.get(&key) {
            // So it is bypassing the actual subgraph request
            return payload.end_with_response(cached_response.clone());
        }
        payload.on_end(move |payload: OnSubgraphHttpResponseHookPayload| {
            self.cache.insert(key, payload.response.clone());
            payload.proceed()
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::testkit::{
        init_graphql_request, init_router_from_config_inline_with_plugins, wait_for_readiness,
        SubgraphsServer,
    };
    use hive_router::PluginRegistry;
    use ntex::web::test;

    // Tests on_subgraph_execute's override behavior
    #[ntex::test]
    async fn caches_subgraph_responses() {
        let subgraphs = SubgraphsServer::start().await;
        let app = init_router_from_config_inline_with_plugins(
            r#"
            plugins:
                subgraph_response_cache:
                    enabled: true
            "#,
            PluginRegistry::new().register::<super::SubgraphResponseCachePlugin>(),
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
