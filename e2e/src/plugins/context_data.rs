// From https://github.com/apollographql/router/blob/dev/examples/context/rust/src/context_data.rs

use serde::Deserialize;

use hive_router_plan_executor::{
    hooks::{
        on_graphql_params::{OnGraphQLParamsStartHookPayload, OnGraphQLParamsStartHookResult},
        on_subgraph_execute::{
            OnSubgraphExecuteEndHookPayload, OnSubgraphExecuteStartHookPayload,
            OnSubgraphExecuteStartHookResult,
        },
    },
    plugin_trait::{EndHookPayload, RouterPlugin, StartHookPayload},
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

#[async_trait::async_trait]
impl RouterPlugin for ContextDataPlugin {
    type Config = ContextDataPluginConfig;
    fn plugin_name() -> &'static str {
        "context_data"
    }
    fn from_config(config: ContextDataPluginConfig) -> Option<Self> {
        if config.enabled {
            Some(ContextDataPlugin {})
        } else {
            None
        }
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
            let context_data = payload.context.get_mut::<ContextData>();
            if let Some(mut context_data) = context_data {
                context_data.response_count += 1;
                tracing::info!("subrequest count {}", context_data.response_count);
            }
            payload.cont()
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
                tracing::info!("subrequest count {}", context_data.response_count);
            }
            payload.cont()
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::testkit::{init_router_from_config_inline, wait_for_readiness, SubgraphsServer};
    use hive_router::PluginRegistry;
    use ntex::web::test;
    #[ntex::test]
    async fn should_add_context_data_and_modify_subgraph_request() {
        let subgraphs = SubgraphsServer::start().await;

        let app = init_router_from_config_inline(
            r#"
            plugins:
              context_data:
                enabled: true
            "#,
            Some(PluginRegistry::new().register::<super::ContextDataPlugin>()),
        )
        .await
        .expect("Router should initialize successfully");

        wait_for_readiness(&app.app).await;

        let resp = test::call_service(
            &app.app,
            crate::testkit::init_graphql_request("{ users { id } }", None).to_request(),
        )
        .await;

        assert!(resp.status().is_success(), "Expected 200 OK");

        let request_logs = subgraphs
            .get_subgraph_requests_log("accounts")
            .await
            .expect("expected requests sent to accounts subgraph");
        assert_eq!(
            request_logs.len(),
            1,
            "expected 1 request to accounts subgraph"
        );
        let hello_header_value = request_logs[0]
            .headers
            .get("x-hello")
            .expect("expected x-hello header to be present in subgraph request")
            .to_str()
            .expect("header value should be valid string");
        assert_eq!(hello_header_value, "Hello world");
    }
}
