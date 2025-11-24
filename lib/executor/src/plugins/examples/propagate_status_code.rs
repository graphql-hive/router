// From https://github.com/apollographql/router/blob/dev/examples/status-code-propagation/rust/src/propagate_status_code.rs

use http::StatusCode;
use serde::Deserialize;

use crate::{
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
        "propagate_status_code_plugin"
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
                let mut ctx_entry = payload.context.get_mut_entry();
                let ctx: Option<&mut PropagateStatusCodeCtx> = ctx_entry.get_ref_mut();
                if let Some(ctx) = ctx {
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
            let ctx_entry = payload.context.get_ref_entry();
            let ctx: Option<&PropagateStatusCodeCtx> = ctx_entry.get_ref();
            if let Some(ctx) = ctx {
                // Update the HTTP response status code
                *payload.response.response_mut().status_mut() = ctx.status_code;
            }
            payload.cont()
        })
    }
}
