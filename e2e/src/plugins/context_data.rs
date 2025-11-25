// From https://github.com/apollographql/router/blob/dev/examples/context/rust/src/context_data.rs

use serde::Deserialize;

use hive_router_plan_executor::{
    hooks::{
        on_graphql_params::{OnGraphQLParamsEndPayload, OnGraphQLParamsStartPayload},
        on_subgraph_execute::{OnSubgraphExecuteEndPayload, OnSubgraphExecuteStartPayload},
    },
    plugin_context::PluginContextMutEntry,
    plugin_trait::{EndPayload, HookResult, RouterPlugin, RouterPluginWithConfig, StartPayload},
};

#[derive(Deserialize)]
pub struct ContextDataPluginConfig {
    pub enabled: bool,
}

pub struct ContextDataPlugin {}

pub struct ContextData {
    incoming_data: String,
    response_count: u64,
}

impl RouterPluginWithConfig for ContextDataPlugin {
    type Config = ContextDataPluginConfig;
    fn plugin_name() -> &'static str {
        "context_data_plugin"
    }
    fn from_config(config: ContextDataPluginConfig) -> Option<Self> {
        if config.enabled {
            Some(ContextDataPlugin {})
        } else {
            None
        }
    }
}

#[async_trait::async_trait]
impl RouterPlugin for ContextDataPlugin {
    async fn on_graphql_params<'exec>(
        &'exec self,
        payload: OnGraphQLParamsStartPayload<'exec>,
    ) -> HookResult<'exec, OnGraphQLParamsStartPayload<'exec>, OnGraphQLParamsEndPayload> {
        let context_data = ContextData {
            incoming_data: "world".to_string(),
            response_count: 0,
        };

        payload.context.insert(context_data);

        payload.on_end(|payload| {
            let mut ctx_data_entry = payload.context.get_mut_entry();
            let context_data: Option<&mut ContextData> = ctx_data_entry.get_ref_mut();
            if let Some(context_data) = context_data {
                context_data.response_count += 1;
                tracing::info!("subrequest count {}", context_data.response_count);
            }
            payload.cont()
        })
    }
    async fn on_subgraph_execute<'exec>(
        &'exec self,
        mut payload: OnSubgraphExecuteStartPayload<'exec>,
    ) -> HookResult<'exec, OnSubgraphExecuteStartPayload<'exec>, OnSubgraphExecuteEndPayload> {
        let ctx_data_entry = payload.context.get_ref_entry();
        let context_data: Option<&ContextData> = ctx_data_entry.get_ref();
        if let Some(context_data) = context_data {
            tracing::info!("hello {}", context_data.incoming_data); // Hello world!
            let new_header_value = format!("Hello {}", context_data.incoming_data);
            payload.execution_request.headers.insert(
                "x-hello",
                http::HeaderValue::from_str(&new_header_value).unwrap(),
            );
        }
        payload.on_end(|payload: OnSubgraphExecuteEndPayload<'exec>| {
            let mut ctx_data_entry: PluginContextMutEntry<ContextData> =
                payload.context.get_mut_entry();
            let context_data: Option<&mut ContextData> = ctx_data_entry.get_ref_mut();
            if let Some(context_data) = context_data {
                context_data.response_count += 1;
                tracing::info!("subrequest count {}", context_data.response_count);
            }
            payload.cont()
        })
    }
}
