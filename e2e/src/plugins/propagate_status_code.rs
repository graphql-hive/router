// From https://github.com/apollographql/router/blob/dev/examples/status-code-propagation/rust/src/propagate_status_code.rs

use http::StatusCode;
use serde::Deserialize;

use hive_router_plan_executor::{
    hooks::{
        on_http_request::{OnHttpRequestPayload, OnHttpResponsePayload},
        on_subgraph_execute::{OnSubgraphExecuteEndPayload, OnSubgraphExecuteStartPayload},
    },
    plugin_trait::{EndPayload, HookResult, RouterPlugin, RouterPluginWithConfig, StartPayload},
};

#[derive(Deserialize)]
pub struct PropagateStatusCodePluginConfig {
    pub enabled: bool,
    pub status_codes: Vec<u64>,
}

impl RouterPluginWithConfig for PropagateStatusCodePlugin {
    type Config = PropagateStatusCodePluginConfig;
    fn plugin_name() -> &'static str {
        "propagate_status_code"
    }
    fn from_config(config: PropagateStatusCodePluginConfig) -> Option<Self> {
        if !config.enabled {
            return None;
        }
        let status_codes = config
            .status_codes
            .into_iter()
            .filter_map(|code| StatusCode::from_u16(code as u16).ok())
            .collect();
        Some(PropagateStatusCodePlugin { status_codes })
    }
}

pub struct PropagateStatusCodePlugin {
    pub status_codes: Vec<StatusCode>,
}

pub struct PropagateStatusCodeCtx {
    pub status_code: StatusCode,
}

#[async_trait::async_trait]
impl RouterPlugin for PropagateStatusCodePlugin {
    async fn on_subgraph_execute<'exec>(
        &'exec self,
        payload: OnSubgraphExecuteStartPayload<'exec>,
    ) -> HookResult<'exec, OnSubgraphExecuteStartPayload<'exec>, OnSubgraphExecuteEndPayload<'exec>>
    {
        payload.on_end(|payload| {
            let status_code = payload.execution_result.status;
            // if a response contains a status code we're watching...
            if self.status_codes.contains(&status_code) {
                // Checking if there is already a context entry
                let ctx = payload.context.get_mut::<PropagateStatusCodeCtx>();
                if let Some(mut ctx) = ctx {
                    // Update the status code if the new one is more severe (higher)
                    if status_code.as_u16() > ctx.status_code.as_u16() {
                        ctx.status_code = status_code;
                    }
                } else {
                    // Insert a new context entry
                    let new_ctx = PropagateStatusCodeCtx { status_code };
                    payload.context.insert(new_ctx);
                }
            }
            payload.cont()
        })
    }
    fn on_http_request<'exec>(
        &'exec self,
        payload: OnHttpRequestPayload<'exec>,
    ) -> HookResult<'exec, OnHttpRequestPayload<'exec>, OnHttpResponsePayload<'exec>> {
        payload.on_end(|mut payload| {
            // Checking if there is a context entry
            let ctx = payload.context.get_ref::<PropagateStatusCodeCtx>();
            if let Some(ctx) = ctx {
                // Update the HTTP response status code
                *payload.response.response_mut().status_mut() = ctx.status_code;
            }
            payload.cont()
        })
    }
}

#[cfg(test)]
mod tests {
    #[ntex::test]
    async fn propagates_highest_status_code() {
        let mut subgraphs_server = mockito::Server::new_async().await;
        let accounts_mock_207 = subgraphs_server
            .mock("POST", "/accounts")
            .with_status(207)
            .with_body(r#"{"data": {"users": [{"id": "1"}]}}"#)
            .create_async()
            .await;
        let products_mock_206 = subgraphs_server
            .mock("POST", "/products")
            .with_status(206)
            .with_body(r#"{"data": {"topProducts": [{"upc": "a"}]}}"#)
            .create_async()
            .await;
        let app = crate::testkit::init_router_from_config_inline(
            &format!(
                r#"
                override_subgraph_urls:
                    accounts:
                        url: http://{}/accounts
                    products:
                        url: http://{}/products
                plugins:
                  propagate_status_code:
                    enabled: true
                    status_codes: [206, 207]
            "#,
                subgraphs_server.host_with_port(),
                subgraphs_server.host_with_port()
            ),
            Some(hive_router::PluginRegistry::new().register::<super::PropagateStatusCodePlugin>()),
        )
        .await
        .expect("failed to start router");
        crate::testkit::wait_for_readiness(&app.app).await;

        let req =
            crate::testkit::init_graphql_request("{ users { id } topProducts { upc } }", None);
        let resp = ntex::web::test::call_service(&app.app, req.to_request()).await;
        assert_eq!(resp.status().as_u16(), 207);
        accounts_mock_207.assert_async().await;
        products_mock_206.assert_async().await;
    }
    #[ntex::test]
    async fn ignores_unlisted_status_codes() {
        let mut subgraphs_server = mockito::Server::new_async().await;
        let accounts_mock_208 = subgraphs_server
            .mock("POST", "/accounts")
            .with_status(208)
            .with_body(r#"{"data": {"users": [{"id": "1"}]}}"#)
            .create_async()
            .await;
        let products_mock_209 = subgraphs_server
            .mock("POST", "/products")
            .with_status(209)
            .with_body(r#"{"data": {"topProducts": [{"upc": "a"}]}}"#)
            .create_async()
            .await;
        let app = crate::testkit::init_router_from_config_inline(
            &format!(
                r#"
                override_subgraph_urls:
                    accounts:
                        url: http://{}/accounts
                    products:
                        url: http://{}/products
                plugins:
                  propagate_status_code:
                    enabled: true
                    status_codes: [208]
            "#,
                subgraphs_server.host_with_port(),
                subgraphs_server.host_with_port()
            ),
            Some(hive_router::PluginRegistry::new().register::<super::PropagateStatusCodePlugin>()),
        )
        .await
        .expect("failed to start router");
        crate::testkit::wait_for_readiness(&app.app).await;
        let req =
            crate::testkit::init_graphql_request("{ users { id } topProducts { upc } }", None);
        let resp = ntex::web::test::call_service(&app.app, req.to_request()).await;
        assert_eq!(resp.status().as_u16(), 208);
        accounts_mock_208.assert_async().await;
        products_mock_209.assert_async().await;
    }
}
