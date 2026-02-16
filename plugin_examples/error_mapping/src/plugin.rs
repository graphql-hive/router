// From https://github.com/apollographql/router/blob/dev/examples/context/rust/src/context_data.rs

use hive_router::{
    async_trait,
    http::status,
    ntex::util::HashMap,
    plugins::{
        hooks::{
            on_graphql_error::{OnGraphQLErrorHookPayload, OnGraphQLErrorHookResult},
            on_plugin_init::{OnPluginInitPayload, OnPluginInitResult},
        },
        plugin_trait::RouterPlugin,
    },
    tracing,
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
    fn on_graphql_error(&self, mut payload: OnGraphQLErrorHookPayload) -> OnGraphQLErrorHookResult {
        if let Some(code) = &payload.error.extensions.code {
            if let Some(mapping) = self.config.get(code) {
                if let Some(status_code) = mapping.status_code {
                    if let Ok(status_code) = status::StatusCode::from_u16(status_code) {
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
                    payload.error.extensions.code = Some(new_code.clone());
                }
            }
        }
        payload.proceed()
    }
}
