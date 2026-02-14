use std::collections::HashMap;

use hive_router::{
    http::header::CONTENT_TYPE,
    ntex::http::header::HeaderValue,
    plugins::{
        hooks::{
            on_http_request::{OnHttpRequestHookPayload, OnHttpRequestHookResult},
            on_plugin_init::{OnPluginInitPayload, OnPluginInitResult},
        },
        plugin_trait::{RouterPlugin, StartHookPayload},
    },
};

use serde::{Deserialize, Serialize};

#[derive(Default, Serialize, Deserialize, Debug, Clone)]
#[serde(default)]
pub struct NonStandardRequestPluginConfig {
    content_type_map: HashMap<String, String>,
}
pub struct NonStandardRequestPlugin {
    content_type_map: HashMap<String, String>,
}

impl RouterPlugin for NonStandardRequestPlugin {
    type Config = NonStandardRequestPluginConfig;
    fn plugin_name() -> &'static str {
        "non_standard_request"
    }
    fn on_plugin_init(payload: OnPluginInitPayload<Self>) -> OnPluginInitResult<Self> {
        let config = payload.config()?;
        // Lowercase the headers
        payload.initialize_plugin(Self {
            content_type_map: config
                .content_type_map
                .into_iter()
                .map(|(k, v)| (k.to_lowercase(), v))
                .collect(),
        })
    }
    fn on_http_request<'req>(
        &'req self,
        mut payload: OnHttpRequestHookPayload<'req>,
    ) -> OnHttpRequestHookResult<'req> {
        let mapped_content_type = payload
            .router_http_request
            .headers()
            .get(CONTENT_TYPE)
            // If it is a valid UTF-8
            .and_then(|header| header.to_str().ok())
            // Lowercase the header value
            .map(|header_str| header_str.to_lowercase())
            // Map to the new content type if it exists in the map
            .and_then(|lowercased_header_str| {
                // Check for the beginning, because it can be like x/y;charset=utf-8
                self.content_type_map
                    .iter()
                    .find(|(k, _)| lowercased_header_str.starts_with(k.as_str()))
                    .map(|(found_part, mapped_content_type_str)| {
                        // Only replace that part, keep the charset if exists
                        lowercased_header_str.replacen(found_part, mapped_content_type_str, 1)
                    })
            })
            // Convert the mapped content type to a HeaderValue
            .and_then(|mapped_content_type_str| {
                HeaderValue::from_str(&mapped_content_type_str).ok()
            });
        // If there is a mapped content type
        if let Some(mapped_content_type) = mapped_content_type {
            // Replace the content type header with the mapped content type
            payload
                .router_http_request
                .headers_mut()
                .insert(CONTENT_TYPE, mapped_content_type);
        }
        payload.proceed()
    }
}
