use hive_router::plugins::hooks::on_http_request::{
    OnHttpRequestHookPayload, OnHttpRequestHookResult,
};
use hive_router::plugins::hooks::on_plugin_init::{OnPluginInitPayload, OnPluginInitResult};
use hive_router::plugins::plugin_trait::{EndHookPayload, RouterPlugin, StartHookPayload};
use hive_router::{
    async_trait, get_current_summary, set_log_correlation, set_summary_message, tracing,
};
use std::sync::atomic::Ordering::Relaxed;
use std::time::Instant;

pub struct CustomLoggerCorrelationPlugin;

const PROJECT_ID_KEY: &str = "project_id";

#[async_trait]
impl RouterPlugin for CustomLoggerCorrelationPlugin {
    type Config = ();

    fn plugin_name() -> &'static str {
        "custom_logger_correlation"
    }

    fn on_plugin_init(payload: OnPluginInitPayload<Self>) -> OnPluginInitResult<Self> {
        payload.initialize_plugin(Self)
    }

    fn on_http_request<'req>(
        &'req self,
        payload: OnHttpRequestHookPayload<'req>,
    ) -> OnHttpRequestHookResult<'req> {
        let project_id = payload
            .router_http_request
            .path()
            .split('/')
            .nth(1)
            .filter(|segment| !segment.is_empty())
            .unwrap_or("unknown_project")
            .to_string();

        let started_at = Instant::now();

        // Attach it as soon as we know it, so every log line for the rest of this
        // request - including this plugin's own - carries the same correlation.
        set_log_correlation(PROJECT_ID_KEY, project_id);

        tracing::debug!(target: "custom_logger_correlation", "on_http_request called");

        let method = payload.router_http_request.method().to_string();
        let path = payload.router_http_request.path().to_string();

        payload.on_end(move |end_payload| {
            let current_summary = if let Some(summary) = get_current_summary() {
                summary
            } else {
                return end_payload.proceed();
            };

            let operation_name = current_summary.operation_name.get().cloned();
            let status_code = current_summary.status_code.load(Relaxed);

            set_summary_message(format!(
                "[status={}] [{}ms] {} {} {}",
                status_code,
                started_at.elapsed().as_millis(),
                method,
                path,
                operation_name.as_deref().unwrap_or("-")
            ));

            end_payload.proceed()
        })
    }
}
