use std::sync::Arc;

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
    registered_plugins: Vec<(&'static str, PluginFactory)>,
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
            registered_plugins: Vec::new(),
        }
    }
    pub fn register<P: RouterPlugin>(mut self) -> Self {
        let plugin_name = P::plugin_name();
        self.registered_plugins.push((
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
        ));
        self
    }
    pub fn initialize_plugins(
        &self,
        router_config: &HiveRouterConfig,
        bg_tasks_manager: &mut BackgroundTasksManager,
    ) -> Result<Option<Arc<Vec<RouterPluginBoxed>>>, PluginRegistryError> {
        let mut plugins_unordered = Vec::with_capacity(router_config.plugins.len());

        for (plugin_name, plugin_config_value) in router_config.plugins.iter() {
            if !plugin_config_value.enabled {
                continue;
            }
            if let Some(factory) = self
                .registered_plugins
                .iter()
                .find_map(|(name, factory)| (*name == plugin_name).then_some(factory))
            {
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
                            plugins_unordered.push((plugin_name.as_str(), plugin));
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

        let mut plugins_ordered = Vec::with_capacity(plugins_unordered.len());
        // Plugins should be ordered by its order in the registration
        for (plugin_name, _factory) in self.registered_plugins.iter() {
            let position = plugins_unordered
                .iter()
                .position(|(name, _factory)| name == plugin_name);
            if let Some(position) = position {
                let (_name, plugin) = plugins_unordered.remove(position);
                plugins_ordered.push(plugin);
            }
        }

        if plugins_ordered.is_empty() {
            Ok(None)
        } else {
            Ok(Some(plugins_ordered.into()))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use hive_router_config::{HiveRouterConfig, PluginConfig};
    use hive_router_internal::background_tasks::BackgroundTasksManager;
    use hive_router_plan_executor::{
        hooks::{
            on_graphql_params::{
                GraphQLParams, OnGraphQLParamsStartHookPayload, OnGraphQLParamsStartHookResult,
            },
            on_plugin_init::{OnPluginInitPayload, OnPluginInitResult},
        },
        plugin_context::{PluginContext, RouterHttpRequest},
        plugin_trait::{RouterPlugin, StartHookPayload},
    };
    use ntex::router::Path;

    use crate::PluginRegistry;

    #[ntex::test]
    async fn keeps_the_order_of_registration() {
        #[derive(Default)]
        struct TestPlugin1;
        #[async_trait::async_trait]
        impl RouterPlugin for TestPlugin1 {
            type Config = ();
            fn plugin_name() -> &'static str {
                "TestPlugin1"
            }
            fn on_plugin_init(payload: OnPluginInitPayload<Self>) -> OnPluginInitResult<Self> {
                payload.initialize_plugin_with_defaults()
            }
            async fn on_graphql_params<'exec>(
                &'exec self,
                payload: OnGraphQLParamsStartHookPayload<'exec>,
            ) -> OnGraphQLParamsStartHookResult<'exec> {
                payload
                    .with_graphql_params(GraphQLParams {
                        query: Some("TestPlugin1".into()),
                        ..Default::default()
                    })
                    .proceed()
            }
        }
        #[derive(Default)]
        struct TestPlugin2;
        #[async_trait::async_trait]
        impl RouterPlugin for TestPlugin2 {
            type Config = ();
            fn plugin_name() -> &'static str {
                "TestPlugin2"
            }
            fn on_plugin_init(payload: OnPluginInitPayload<Self>) -> OnPluginInitResult<Self> {
                payload.initialize_plugin_with_defaults()
            }
            async fn on_graphql_params<'exec>(
                &'exec self,
                payload: OnGraphQLParamsStartHookPayload<'exec>,
            ) -> OnGraphQLParamsStartHookResult<'exec> {
                payload
                    .with_graphql_params(GraphQLParams {
                        query: Some("TestPlugin2".into()),
                        ..Default::default()
                    })
                    .proceed()
            }
        }
        #[derive(Default)]
        struct TestPlugin3;
        #[async_trait::async_trait]
        impl RouterPlugin for TestPlugin3 {
            type Config = ();
            fn plugin_name() -> &'static str {
                "TestPlugin3"
            }
            fn on_plugin_init(payload: OnPluginInitPayload<Self>) -> OnPluginInitResult<Self> {
                payload.initialize_plugin_with_defaults()
            }
            async fn on_graphql_params<'exec>(
                &'exec self,
                payload: OnGraphQLParamsStartHookPayload<'exec>,
            ) -> OnGraphQLParamsStartHookResult<'exec> {
                payload
                    .with_graphql_params(GraphQLParams {
                        query: Some("TestPlugin3".into()),
                        ..Default::default()
                    })
                    .proceed()
            }
        }

        let registry = PluginRegistry::new()
            .register::<TestPlugin1>()
            .register::<TestPlugin2>()
            .register::<TestPlugin3>();

        let bg_tasks_manager = &mut BackgroundTasksManager::default();
        let mut router_config = HiveRouterConfig::default();
        router_config.plugins = HashMap::from_iter(
            vec![
                (
                    "TestPlugin2".into(),
                    PluginConfig {
                        enabled: true,
                        ..Default::default()
                    },
                ),
                (
                    "TestPlugin3".into(),
                    PluginConfig {
                        enabled: true,
                        ..Default::default()
                    },
                ),
                (
                    "TestPlugin1".into(),
                    PluginConfig {
                        enabled: true,
                        ..Default::default()
                    },
                ),
            ]
            .into_iter(),
        );
        let plugins = registry
            .initialize_plugins(&router_config, bg_tasks_manager)
            .expect("Plugins should be initialized successfully")
            .expect("Plugins should exist");
        let uri: http::Uri = "http://example.com/graphql".parse().unwrap();
        let path: Path<http::Uri> = Path::new(uri.clone());
        let fake_request = RouterHttpRequest {
            uri: &uri,
            method: &http::Method::POST,
            version: http::Version::HTTP_11,
            headers: &ntex::http::HeaderMap::new(),
            path: "/graphql",
            query_string: "",
            match_info: &path,
        };
        let plugin_context = PluginContext::default();
        let mut plugin_names: Vec<String> = vec![];
        for plugin in plugins.iter() {
            let payload = OnGraphQLParamsStartHookPayload {
                router_http_request: &fake_request,
                context: &plugin_context,
                body: Default::default(),
                graphql_params: None,
            };
            let result = plugin.on_graphql_params(payload).await;
            plugin_names.push(
                result
                    .payload
                    .graphql_params
                    .unwrap()
                    .query
                    .unwrap()
                    .to_string(),
            );
        }
        assert_eq!(
            plugin_names,
            vec!["TestPlugin1", "TestPlugin2", "TestPlugin3"]
        );
    }
}
