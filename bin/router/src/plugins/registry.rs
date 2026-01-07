use std::collections::HashMap;

use hive_router_config::HiveRouterConfig;
use hive_router_plan_executor::plugin_trait::{RouterPlugin, RouterPluginBoxed};

use crate::BoxError;

type PluginFactory =
    Box<dyn Fn(serde_json::Value) -> Result<Option<RouterPluginBoxed>, PluginRegistryError>>;

pub struct PluginRegistry {
    map: HashMap<&'static str, PluginFactory>,
}

impl Default for PluginRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PluginRegistryError {
    #[error("Failed to parse the configuration for the plugin '{0}': {1}")]
    Config(&'static str, serde_json::Error),
    #[error("Failed to initialize the plugin '{0}': {1}")]
    Initialization(&'static str, BoxError),
    #[error(
        "Plugin '{0}' is not registered in the registry but is specified in the configuration"
    )]
    MissingInRegistry(String),
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
        }
    }
    pub fn register<P: RouterPlugin>(mut self) -> Self {
        let plugin_name = P::plugin_name();
        self.map.insert(
            plugin_name,
            Box::new(|plugin_config: serde_json::Value| {
                let config: P::Config = serde_json::from_value(plugin_config)
                    .map_err(|err| PluginRegistryError::Config(plugin_name, err))?;
                let plugin = P::from_config(config)
                    .map_err(|err| PluginRegistryError::Initialization(plugin_name, err))?;
                Ok(Option::map(plugin, |p| Box::new(p) as RouterPluginBoxed))
            }),
        );
        self
    }
    pub fn initialize_plugins(
        &self,
        router_config: &HiveRouterConfig,
    ) -> Result<Option<Vec<RouterPluginBoxed>>, PluginRegistryError> {
        let mut plugins: Vec<RouterPluginBoxed> = vec![];

        for (plugin_name, plugin_config_value) in router_config.plugins.iter() {
            if let Some(factory) = self.map.get(plugin_name.as_str()) {
                let plugin = factory(plugin_config_value.clone())?;
                if let Some(p) = plugin {
                    plugins.push(p);
                }
            } else {
                return Err(PluginRegistryError::MissingInRegistry(
                    plugin_name.to_string(),
                ));
            }
        }

        if plugins.is_empty() {
            Ok(None)
        } else {
            Ok(Some(plugins))
        }
    }
}
