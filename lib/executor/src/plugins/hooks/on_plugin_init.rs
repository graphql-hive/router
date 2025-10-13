use std::error::Error;

use hive_router_internal::{
    background_tasks::{BackgroundTask, BackgroundTasksManager},
    BoxError,
};

use crate::plugin_trait::RouterPlugin;

pub struct OnPluginInitPayload<'a, TRouterPlugin: RouterPlugin> {
    config: &'a serde_json::Value,
    bg_tasks_manager: &'a mut BackgroundTasksManager,
    phantom: std::marker::PhantomData<TRouterPlugin>,
}

pub type OnPluginInitResult<TRouterPlugin> = Result<Option<TRouterPlugin>, Box<dyn Error>>;

impl<'a, TRouterPlugin> OnPluginInitPayload<'a, TRouterPlugin>
where
    TRouterPlugin: RouterPlugin,
{
    pub fn new(
        config: &'a serde_json::Value,
        bg_tasks_manager: &'a mut BackgroundTasksManager,
    ) -> Self {
        Self {
            config,
            bg_tasks_manager,
            phantom: std::marker::PhantomData,
        }
    }

    // Parse configuration when needed
    pub fn config(&self) -> Result<TRouterPlugin::Config, Box<dyn Error>> {
        let sonic_value = sonic_rs::to_value(self.config)?;
        let config = sonic_rs::from_value(&sonic_value)?;
        Ok(config)
    }

    pub fn register_background_task<T>(&mut self, task: T)
    where
        T: BackgroundTask + 'static,
    {
        self.bg_tasks_manager.register_task(task)
    }
    pub fn disable_plugin(&self) -> OnPluginInitResult<TRouterPlugin> {
        Ok(None)
    }
    pub fn initialize_plugin(&self, plugin: TRouterPlugin) -> OnPluginInitResult<TRouterPlugin> {
        Ok(Some(plugin))
    }
    pub fn initialize_plugin_with_defaults(&self) -> OnPluginInitResult<TRouterPlugin>
    where
        TRouterPlugin: Default,
    {
        Ok(Some(TRouterPlugin::default()))
    }
    pub fn error<TError>(err: TError) -> OnPluginInitResult<TRouterPlugin>
    where
        TError: Error + Into<BoxError>,
    {
        Err(err.into())
    }
}
