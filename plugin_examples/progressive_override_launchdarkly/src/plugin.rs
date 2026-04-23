use std::collections::HashSet;
use std::env::var;

use hive_router::{
    async_trait,
    plugins::{
        hooks::{
            on_plugin_init::{OnPluginInitPayload, OnPluginInitResult},
            on_query_plan::{OnQueryPlanStartHookPayload, OnQueryPlanStartHookResult},
        },
        plugin_trait::{RouterPlugin, StartHookPayload},
    },
    tracing::warn,
};
use launchdarkly_server_sdk::{Client, ConfigBuilder, Context, ContextBuilder};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct ProgressiveOverrideLaunchDarklyConfig {
    #[serde(default)]
    pub context_key_header: Option<String>,
}

pub struct ProgressiveOverrideLaunchDarklyPlugin {
    client: Client,
    context_key_header: String,
}

#[async_trait]
impl RouterPlugin for ProgressiveOverrideLaunchDarklyPlugin {
    type Config = ProgressiveOverrideLaunchDarklyConfig;

    fn plugin_name() -> &'static str {
        "progressive_override_launchdarkly"
    }

    fn on_plugin_init(payload: OnPluginInitPayload<Self>) -> OnPluginInitResult<Self> {
        let config = payload.config()?;
        let sdk_key = var("LD_SDK_KEY")?;

        let ld_config = ConfigBuilder::new(sdk_key.as_str()).build()?;
        let client = Client::build(ld_config)?;
        client.start_with_default_executor();

        payload.initialize_plugin(Self {
            client,
            context_key_header: config
                .context_key_header
                .unwrap_or_else(|| "x-user-id".to_string()),
        })
    }

    async fn on_query_plan<'exec>(
        &'exec self,
        start_payload: OnQueryPlanStartHookPayload<'exec>,
    ) -> OnQueryPlanStartHookResult<'exec> {
        let snapshot = match start_payload.request_context.read() {
            Ok(snapshot) => snapshot,
            Err(err) => {
                warn!(error = %err, "failed to read request context");
                return start_payload.proceed();
            }
        };

        let progressive_override = snapshot.progressive_override();
        let Some(unresolved_labels) = progressive_override.unresolved_labels() else {
            return start_payload.proceed();
        };

        if unresolved_labels.is_empty() {
            return start_payload.proceed();
        }

        let context = build_ld_context(&start_payload, &self.context_key_header);
        let mut labels_to_override = HashSet::new();

        for label in unresolved_labels {
            let is_enabled = self.client.bool_variation(&context, label, false);

            if is_enabled {
                labels_to_override.insert(label.clone());
            }
        }

        if let Ok(mut write) = start_payload.request_context.write() {
            write
                .progressive_override()
                .set_labels_to_override(Some(labels_to_override));
        }

        start_payload.proceed()
    }

    async fn on_shutdown<'exec>(&'exec self) {
        self.client.close();
    }
}

fn build_ld_context(
    payload: &OnQueryPlanStartHookPayload<'_>,
    context_key_header: &str,
) -> Context {
    let context_key = payload
        .router_http_request
        .headers
        .get(context_key_header)
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty())
        .unwrap_or("anonymous");

    ContextBuilder::new(context_key)
        .build()
        .unwrap_or_else(|_| {
            ContextBuilder::new("anonymous")
                .build()
                .expect("valid context")
        })
}
