// From https://github.com/apollographql/router/blob/dev/examples/status-code-propagation/rust/src/propagate_status_code.rs

use hive_router::http::StatusCode;
use serde::Deserialize;

use hive_router::plugins::{
    hooks::{
        on_http_request::{OnHttpRequestHookPayload, OnHttpRequestHookResult},
        on_plugin_init::{OnPluginInitPayload, OnPluginInitResult},
        on_subgraph_http_request::{
            OnSubgraphHttpRequestHookPayload, OnSubgraphHttpRequestHookResult,
        },
    },
    plugin_trait::{EndHookPayload, RouterPlugin, StartHookPayload},
};

#[derive(Deserialize)]
pub struct PropagateStatusCodePluginConfig {
    pub status_codes: Vec<u64>,
}

pub struct PropagateStatusCodePlugin {
    pub status_codes: Vec<StatusCode>,
}

pub struct PropagateStatusCodeCtx {
    pub status_code: StatusCode,
}

#[hive_router::async_trait]
impl RouterPlugin for PropagateStatusCodePlugin {
    type Config = PropagateStatusCodePluginConfig;
    fn plugin_name() -> &'static str {
        "propagate_status_code"
    }
    fn on_plugin_init(payload: OnPluginInitPayload<Self>) -> OnPluginInitResult<Self> {
        let status_codes = payload
            .config()?
            .status_codes
            .iter()
            .filter_map(|code| StatusCode::from_u16(*code as u16).ok())
            .collect();
        payload.initialize_plugin(Self { status_codes })
    }
    async fn on_subgraph_http_request<'exec>(
        &'exec self,
        payload: OnSubgraphHttpRequestHookPayload<'exec>,
    ) -> OnSubgraphHttpRequestHookResult<'exec> {
        payload.on_end(|payload| {
            let status_code = payload.response.status;
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
            payload.proceed()
        })
    }
    fn on_http_request<'exec>(
        &'exec self,
        payload: OnHttpRequestHookPayload<'exec>,
    ) -> OnHttpRequestHookResult<'exec> {
        payload.on_end(|payload| {
            // Checking if there is a context entry
            let ctx = payload.context.get_ref::<PropagateStatusCodeCtx>();
            if let Some(ctx) = ctx {
                // Update the HTTP response status code
                return payload
                    .map_response(|mut response| {
                        *response.response_mut().status_mut() = ctx.status_code;
                        response
                    })
                    .proceed();
            }
            payload.proceed()
        })
    }
}

#[cfg(test)]
mod tests {
    use e2e::{
        mockito,
        testkit::{EnvVarsGuard, TestRouterBuilder},
    };
    use hive_router::ntex;

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

        let _env_guard = EnvVarsGuard::new()
            .set(
                "ACCOUNTS_URL_OVERRIDE",
                &format!("http://{}/accounts", subgraphs_server.host_with_port()),
            )
            .set(
                "PRODUCTS_URL_OVERRIDE",
                &format!("http://{}/products", subgraphs_server.host_with_port()),
            )
            .apply()
            .await;

        let router = TestRouterBuilder::new()
            .file_config("../plugin_examples/propagate_status_code/router.config.yaml")
            .register_plugin::<super::PropagateStatusCodePlugin>()
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request("{ users { id } topProducts { upc } }", None, None)
            .await;
        assert_eq!(res.status().as_u16(), 207);
        accounts_mock_207.assert_async().await;
        products_mock_206.assert_async().await;
    }

    #[ntex::test]
    async fn ignores_unlisted_status_codes() {
        let mut subgraphs_server = mockito::Server::new_async().await;
        let accounts_mock_206 = subgraphs_server
            .mock("POST", "/accounts")
            .with_status(206)
            .with_body(r#"{"data": {"users": [{"id": "1"}]}}"#)
            .create_async()
            .await;
        let products_mock_208 = subgraphs_server
            .mock("POST", "/products")
            .with_status(208)
            .with_body(r#"{"data": {"topProducts": [{"upc": "a"}]}}"#)
            .create_async()
            .await;

        let _env_guard = EnvVarsGuard::new()
            .set(
                "ACCOUNTS_URL_OVERRIDE",
                &format!("http://{}/accounts", subgraphs_server.host_with_port()),
            )
            .set(
                "PRODUCTS_URL_OVERRIDE",
                &format!("http://{}/products", subgraphs_server.host_with_port()),
            )
            .apply()
            .await;

        let router = TestRouterBuilder::new()
            .file_config("../plugin_examples/propagate_status_code/router.config.yaml")
            .register_plugin::<super::PropagateStatusCodePlugin>()
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request("{ users { id } topProducts { upc } }", None, None)
            .await;
        assert_eq!(res.status().as_u16(), 206);
        accounts_mock_206.assert_async().await;
        products_mock_208.assert_async().await;
    }
}
