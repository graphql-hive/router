// From https://github.com/apollographql/router/blob/dev/examples/context/rust/src/context_data.rs

use hive_router::{
    async_trait, http::status, ntex::util::HashMap, plugins::{
        hooks::{
            on_graphql_error::{OnGraphQLErrorHookPayload, OnGraphQLErrorHookResult}, on_http_request::{OnHttpRequestHookPayload, OnHttpRequestHookResult}, on_plugin_init::{OnPluginInitPayload, OnPluginInitResult}
        },
        plugin_trait::{EndHookPayload, RouterPlugin, StartHookPayload},
    }, tracing
};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct ErrorMappingConfig {
    pub status_code: Option<u16>,
    pub code: Option<String>,
}

pub struct ErrorMappingPlugin {
    config: HashMap<String, ErrorMappingConfig>,
}

pub struct ErrorMappingCtx{
    count: usize,
}

#[async_trait]
impl RouterPlugin for ErrorMappingPlugin {
    type Config = HashMap<String, ErrorMappingConfig>;
    fn plugin_name() -> &'static str {
        "error_mapping"
    }
    fn on_plugin_init(payload: OnPluginInitPayload<Self>) -> OnPluginInitResult<Self> {
        let config = payload.config()?;
        payload.initialize_plugin(Self { config })
    }
    fn on_graphql_error<'req>(
        &self,
        mut payload: OnGraphQLErrorHookPayload<'req>,
    ) -> OnGraphQLErrorHookResult<'req> {
        let mut mapped = false;
        if let Some(code) = &payload.error.extensions.code {
            if let Some(mapping) = self.config.get(code) {
                if let Some(status_code) = mapping.status_code {
                    if let Ok(status_code) = status::StatusCode::from_u16(status_code) {
                        mapped = true;
                        payload.status_code = status_code;
                    } else {
                        tracing::error!(
                            "Invalid status code {} for error code {}",
                            status_code,
                            code
                        );
                    }
                }
                if let Some(new_code) = &mapping.code {
                    mapped = true;
                    payload.error.extensions.code = Some(new_code.clone());
                }
            }
        }
        if mapped {
            // Put it in to the context
            let ctx = payload.context.get_mut::<ErrorMappingCtx>();
            if let Some(mut ctx) = ctx {
                ctx.count += 1;
            } else {
                payload.context.insert(ErrorMappingCtx { count: 1 });
            }
        }
        payload.proceed()
    }
    fn on_http_request<'req>(
        &'req self,
        payload: OnHttpRequestHookPayload<'req>,
    ) -> OnHttpRequestHookResult<'req> {
        payload.on_end(move |payload| {
            let mapped_errors_count = payload.context.get_ref::<ErrorMappingCtx>().map(|ctx| ctx.count);
            payload
                .map_response(move |mut response| {
                    if let Some(count) = mapped_errors_count {
                        response.headers_mut().insert(
                            "x-mapped-errors-count".to_string().parse().unwrap(),
                            count.to_string().parse().unwrap(),
                        );
                    }
                    response
                })
                .proceed()
        })
    }
}
