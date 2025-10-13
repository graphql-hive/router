use std::{collections::HashMap, sync::Arc};

use hive_router_config::HiveRouterConfig;
use hive_router_internal::{background_tasks::BackgroundTasksManager, BoxError};
use hive_router_plan_executor::{
    hooks::on_plugin_init::OnPluginInitPayload,
    plugin_trait::{RouterPlugin, RouterPluginBoxed},
};
use tracing::{info, warn};

type PluginFactory = Box<
    dyn Fn(
        &serde_json::Value,
        &mut BackgroundTasksManager,
    ) -> Result<Option<RouterPluginBoxed>, PluginRegistryError>,
>;

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
            Box::new(
                |plugin_config: &serde_json::Value,
                 bg_tasks_manager: &mut BackgroundTasksManager| {
                    let payload = OnPluginInitPayload::new(plugin_config, bg_tasks_manager);
                    let plugin = P::on_plugin_init(payload)
                        .map_err(|err| PluginRegistryError::Initialization(plugin_name, err))?;
                    Ok(Option::map(plugin, |p| Box::new(p) as RouterPluginBoxed))
                },
            ),
        );
        self
    }
    pub fn initialize_plugins(
        &self,
        router_config: &HiveRouterConfig,
        bg_tasks_manager: &mut BackgroundTasksManager,
    ) -> Result<Option<Arc<Vec<RouterPluginBoxed>>>, PluginRegistryError> {
        let mut plugins: Vec<RouterPluginBoxed> = vec![];

        for (plugin_name, plugin_config_value) in router_config.plugins.iter() {
            if let Some(factory) = self.map.get(plugin_name.as_str()) {
                if !plugin_config_value.enabled {
                    continue;
                }
                let plugin_init_result = factory(&plugin_config_value.config, bg_tasks_manager);
                match plugin_init_result {
                    Err(plugin_init_error) => {
                        if plugin_config_value.warn_on_error {
                            warn!("Plugin initialization error: {}", plugin_init_error);
                            continue;
                        } else {
                            return Err(plugin_init_error);
                        }
                    }
                    Ok(maybe_plugin) => {
                        if let Some(plugin) = maybe_plugin {
                            info!("Plugin '{}' successfully enabled", plugin_name);
                            plugins.push(plugin);
                        } else {
                            warn!("Plugin '{}' disabled during initialization", plugin_name);
                        }
                    }
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
            Ok(Some(plugins.into()))
        }
    }
}
