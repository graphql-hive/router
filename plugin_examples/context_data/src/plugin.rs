// From https://github.com/apollographql/router/blob/dev/examples/context/rust/src/context_data.rs

use hive_router::{
    async_trait, http,
    plugins::{
        hooks::{
            on_graphql_params::{OnGraphQLParamsStartHookPayload, OnGraphQLParamsStartHookResult},
            on_plugin_init::{OnPluginInitPayload, OnPluginInitResult},
            on_subgraph_execute::{
                OnSubgraphExecuteEndHookPayload, OnSubgraphExecuteStartHookPayload,
                OnSubgraphExecuteStartHookResult,
            },
        },
        plugin_trait::{EndHookPayload, RouterPlugin, StartHookPayload},
    },
    tracing,
};

pub struct ContextDataPlugin;

pub struct ContextData {
    incoming_data: String,
    response_count: u64,
}

#[async_trait]
impl RouterPlugin for ContextDataPlugin {
    type Config = ();
    fn plugin_name() -> &'static str {
        "context_data"
    }
    fn on_plugin_init(payload: OnPluginInitPayload<Self>) -> OnPluginInitResult<Self> {
        payload.initialize_plugin(Self)
    }
    async fn on_graphql_params<'exec>(
        &'exec self,
        payload: OnGraphQLParamsStartHookPayload<'exec>,
    ) -> OnGraphQLParamsStartHookResult<'exec> {
        let context_data = ContextData {
            incoming_data: "world".to_string(),
            response_count: 0,
        };

        payload.context.insert(context_data);

        payload.on_end(|payload| {
            let context_data = payload.context.get_ref::<ContextData>();
            if let Some(context_data) = context_data {
                tracing::info!("subrequest count {}", context_data.response_count);
            }
            payload.proceed()
        })
    }
    async fn on_subgraph_execute<'exec>(
        &'exec self,
        mut payload: OnSubgraphExecuteStartHookPayload<'exec>,
    ) -> OnSubgraphExecuteStartHookResult<'exec> {
        let context_data_entry = payload.context.get_ref::<ContextData>();
        if let Some(ref context_data_entry) = context_data_entry {
            tracing::info!("hello {}", context_data_entry.incoming_data); // Hello world!
            let new_header_value = format!("Hello {}", context_data_entry.incoming_data);
            payload.execution_request.headers.insert(
                "x-hello",
                http::HeaderValue::from_str(&new_header_value).unwrap(),
            );
        }
        payload.on_end(|payload: OnSubgraphExecuteEndHookPayload<'exec>| {
            let context_data = payload.context.get_mut::<ContextData>();
            if let Some(mut context_data) = context_data {
                context_data.response_count += 1;
            }
            payload.proceed()
        })
    }
}
