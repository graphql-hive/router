use hive_router::plugins::{
    hooks::on_plugin_init::{
        OnPluginInitPayload, OnPluginInitResult, RequestIdentifierExtractionPoint,
    },
    plugin_trait::RouterPlugin,
};

pub struct CustomLoggerCorrelationPlugin;

const MY_CUSTOM_CORRELATOR_KEY: &str = "project_id";

impl RouterPlugin for CustomLoggerCorrelationPlugin {
    type Config = ();
    fn plugin_name() -> &'static str {
        "custom_logger_correlation"
    }

    fn on_plugin_init(mut payload: OnPluginInitPayload<Self>) -> OnPluginInitResult<Self> {
        payload.register_logger_correlation_extractor(|source| match source {
            RequestIdentifierExtractionPoint::Http(req) => {
                let identifier = req
                    .match_info()
                    .get("project_id")
                    .unwrap_or("unknown_project");

                Some(vec![(MY_CUSTOM_CORRELATOR_KEY, identifier.to_string())])
            }
            _ => None,
        });
        payload.initialize_plugin(Self)
    }
}
