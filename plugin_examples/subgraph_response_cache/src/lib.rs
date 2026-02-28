use hive_router::plugins::{
    hooks::{
        on_plugin_init::{OnPluginInitPayload, OnPluginInitResult},
        on_subgraph_http_request::{
            OnSubgraphHttpRequestHookPayload, OnSubgraphHttpRequestHookResult,
            OnSubgraphHttpResponseHookPayload,
        },
    },
    plugin_trait::{EndHookPayload, RouterPlugin, StartHookPayload},
};
use hive_router::{DashMap, SubgraphHttpResponse};

#[derive(Default)]
pub struct SubgraphResponseCachePlugin {
    cache: DashMap<String, SubgraphHttpResponse>,
}

#[hive_router::async_trait]
impl RouterPlugin for SubgraphResponseCachePlugin {
    type Config = ();
    fn plugin_name() -> &'static str {
        "subgraph_response_cache"
    }
    fn on_plugin_init(payload: OnPluginInitPayload<Self>) -> OnPluginInitResult<Self> {
        payload.initialize_plugin_with_defaults()
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
    use e2e::testkit::{TestRouterBuilder, TestSubgraphsBuilder};
    use hive_router::ntex;

    // Tests on_subgraph_execute's override behavior
    #[ntex::test]
    async fn caches_subgraph_responses() {
        let subgraphs = TestSubgraphsBuilder::new().build().start().await;

        let router = TestRouterBuilder::new()
            .with_subgraphs(&subgraphs)
            .file_config("../plugin_examples/subgraph_response_cache/router.config.yaml")
            .register_plugin::<super::SubgraphResponseCachePlugin>()
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request("{ users { id } }", None, None)
            .await;
        assert!(res.status().is_success());

        let res2 = router
            .send_graphql_request("{ users { id } }", None, None)
            .await;
        assert!(res2.status().is_success());

        let subgraph_requests = subgraphs
            .get_requests_log("accounts")
            .expect("failed to get subgraph requests log");
        assert_eq!(subgraph_requests.len(), 1);
    }
}
