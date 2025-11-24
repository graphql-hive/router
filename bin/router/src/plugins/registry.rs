use std::collections::HashMap;

use hive_router_config::HiveRouterConfig;
use hive_router_plan_executor::plugin_trait::{RouterPlugin, RouterPluginWithConfig};
use serde_json::Value;
use tracing::{info, warn};

pub struct PluginRegistry {
    map: HashMap<
        &'static str,
        Box<dyn Fn(Value) -> Result<Box<dyn RouterPlugin + Send + Sync>, serde_json::Error>>,
    >,
}

impl Default for PluginRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
        }
    }
    pub fn register<P: RouterPluginWithConfig + Send + Sync + 'static>(&mut self) {
        self.map.insert(
            P::plugin_name(),
            Box::new(|plugin_config: Value| Ok(P::from_config_value(plugin_config)?)),
        );
    }
    pub fn initialize_plugins(
        &self,
        router_config: &HiveRouterConfig,
    ) -> Vec<Box<dyn RouterPlugin + Send + Sync>> {
        let mut plugins: Vec<Box<dyn RouterPlugin + Send + Sync>> = vec![];

        for (plugin_name, plugin_config_value) in router_config.plugins.iter() {
            if let Some(factory) = self.map.get(plugin_name.as_str()) {
                match factory(plugin_config_value.clone()) {
                    Ok(plugin) => {
                        info!("Loaded plugin: {}", plugin_name);
                        plugins.push(plugin);
                    }
                    Err(err) => {
                        warn!(
                            "Failed to load plugin '{}': {}, skipping plugin",
                            plugin_name, err
                        );
                    }
                }
            } else {
                warn!(
                    "No plugin found registered '{}', skipping plugin",
                    plugin_name
                );
            }
        }
        plugins
    }
}
