use std::collections::HashMap;

use hive_router_config::HiveRouterConfig;
use hive_router_plan_executor::plugin_trait::{RouterPluginBoxed, RouterPluginWithConfig};
use serde_json::Value;
use tracing::info;

type PluginFactory = Box<dyn Fn(Value) -> Result<Option<RouterPluginBoxed>, serde_json::Error>>;

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
    #[error("Failed to initialize plugin '{0}': {1}")]
    Config(String, serde_json::Error),
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
    pub fn register<P: RouterPluginWithConfig + Send + Sync + 'static>(mut self) -> Self {
        self.map.insert(
            P::plugin_name(),
            Box::new(|plugin_config: Value| {
                let config: P::Config = serde_json::from_value(plugin_config)?;
                match P::from_config(config) {
                    Some(plugin) => Ok(Some(Box::new(plugin))),
                    None => Ok(None),
                }
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
                match factory(plugin_config_value.clone()) {
                    Ok(plugin) => {
                        info!("Loaded plugin: {}", plugin_name);
                        match plugin {
                            Some(plugin) => plugins.push(plugin),
                            None => info!("Plugin '{}' is disabled, skipping", plugin_name),
                        }
                    }
                    Err(err) => {
                        return Err(PluginRegistryError::Config(plugin_name.clone(), err));
                    }
                }
            } else {
                return Err(PluginRegistryError::MissingInRegistry(plugin_name.clone()));
            }
        }

        if plugins.is_empty() {
            Ok(None)
        } else {
            Ok(Some(plugins))
        }
    }
}
