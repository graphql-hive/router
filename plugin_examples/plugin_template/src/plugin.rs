use hive_router::{
    async_trait,
    plugins::{
        hooks::{
            on_execute::{OnExecuteStartHookPayload, OnExecuteStartHookResult},
            on_plugin_init::{OnPluginInitPayload, OnPluginInitResult},
            on_subgraph_execute::{
                OnSubgraphExecuteStartHookPayload, OnSubgraphExecuteStartHookResult,
            },
            on_subgraph_http_request::{
                OnSubgraphHttpRequestHookPayload, OnSubgraphHttpRequestHookResult,
            },
        },
        plugin_trait::{RouterPlugin, StartHookPayload},
    },
};

#[derive(Default)]
pub struct MyPlugin;

#[async_trait]
impl RouterPlugin for MyPlugin {
    type Config = ();
    fn plugin_name() -> &'static str {
        "my_plugin"
    }
    fn on_plugin_init(payload: OnPluginInitPayload<Self>) -> OnPluginInitResult<Self> {
        payload.initialize_plugin_with_defaults()
    }
    async fn on_execute<'exec>(
        &'exec self,
        start_payload: OnExecuteStartHookPayload<'exec>,
    ) -> OnExecuteStartHookResult<'exec> {
        start_payload.proceed()
    }

    async fn on_subgraph_execute<'exec>(
        &'exec self,
        start_payload: OnSubgraphExecuteStartHookPayload<'exec>,
    ) -> OnSubgraphExecuteStartHookResult<'exec> {
        start_payload.proceed()
    }

    async fn on_subgraph_http_request<'exec>(
        &'exec self,
        start_payload: OnSubgraphHttpRequestHookPayload<'exec>,
    ) -> OnSubgraphHttpRequestHookResult<'exec> {
        start_payload.proceed()
    }
}
