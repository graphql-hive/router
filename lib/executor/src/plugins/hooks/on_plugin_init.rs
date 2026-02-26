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

    /// Parse the plugin config into the expected config struct for the plugin.
    /// The plugin can choose when and if to call this method.
    ///
    /// [Refer to the docs for more details](https://graphql-hive.com/docs/router/extensibility/plugin_system#configuration)
    ///
    /// Example:
    /// ```rust
    /// fn on_plugin_init(mut payload: OnPluginInitPayload<Self>) -> OnPluginInitResult<Self> {
    ///     let config = payload.config()?;
    ///     // use config to initialize plugin...
    /// }
    /// ```
    pub fn config(&self) -> Result<TRouterPlugin::Config, Box<dyn Error>> {
        let sonic_value = sonic_rs::to_value(self.config)?;
        let config = sonic_rs::from_value(&sonic_value)?;
        Ok(config)
    }

    /// Register a background task to be run by the router.
    /// The registered task struct should implement the `BackgroundTask` trait.
    ///
    /// [Refer to the docs for more details](https://graphql-hive.com/docs/router/extensibility/plugin_system#background-tasks)
    ///
    /// Example:
    /// ```rust
    /// struct MyBackgroundTask {
    ///     // fields for the task...
    /// }
    ///
    /// #[async_trait]
    /// impl BackgroundTask for MyBackgroundTask {
    ///     fn id(&self) -> &str {
    ///         "my_background_task"
    ///     }
    ///     async fn run(&self, token: CancellationToken) {
    ///         loop {
    ///           if token.is_cancelled() {
    ///             break;
    ///          }
    ///         // do background work...
    ///        }
    ///     }
    /// }
    ///
    /// impl RouterPlugin for MyPlugin {
    ///     // ...
    ///     fn on_plugin_init(mut payload: OnPluginInitPayload<Self>) -> OnPluginInitResult<Self> {
    ///         payload.register_background_task(MyBackgroundTask {
    ///             // initialize task fields...
    ///         });
    ///         // initialize plugin...
    ///     }
    /// }
    /// ```
    pub fn register_background_task<T>(&mut self, task: T)
    where
        T: BackgroundTask + 'static,
    {
        self.bg_tasks_manager.register_task(task)
    }
    /// Returning this will disable the plugin and it won't be initialized.
    /// This can be used if the plugin determines during initialization that it shouldn't run
    /// (e.g. due to missing configuration or environment variables).
    ///
    /// Example:
    /// ```rust
    /// fn on_plugin_init(payload: OnPluginInitPayload<Self>) -> OnPluginInitResult {
    ///     if some_condition {
    ///        return payload.disable_plugin();
    ///     }
    ///     // continue with initialization...
    /// }
    /// ```
    pub fn disable_plugin(&self) -> OnPluginInitResult<TRouterPlugin> {
        Ok(None)
    }
    /// Returning this will initialize the plugin with the provided instance.
    /// Example:
    /// ```rust
    /// fn on_plugin_init(mut payload: OnPluginInitPayload<Self>) -> OnPluginInitResult<Self> {
    ///     let config = payload.config()?;
    ///     let plugin_instance = Self {
    ///         // initialize plugin fields from config...
    ///     };
    ///     payload.initialize_plugin(plugin_instance)
    /// }
    /// ```
    pub fn initialize_plugin(&self, plugin: TRouterPlugin) -> OnPluginInitResult<TRouterPlugin> {
        Ok(Some(plugin))
    }
    /// If the plugin struct implements `Default`, this method can be used to initialize the plugin with default values.
    /// Example:
    /// ```rust
    /// #[derive(Default)]
    /// struct MyPlugin {
    ///   values: Vec<String>,
    /// }
    ///
    /// impl RouterPlugin for MyPlugin {
    ///     // ...
    ///     fn on_plugin_init(payload: OnPluginInitPayload<Self>) -> OnPluginInitResult<Self> {
    ///        payload.initialize_plugin_with_defaults()
    ///     }
    /// }
    /// ```
    pub fn initialize_plugin_with_defaults(&self) -> OnPluginInitResult<TRouterPlugin>
    where
        TRouterPlugin: Default,
    {
        Ok(Some(TRouterPlugin::default()))
    }
    /// Returning an error from this method will cause the router to fail initialization and the error will be logged.
    /// This can be used if the plugin encounters an unrecoverable error during initialization.
    /// Example:
    /// ```rust
    /// fn on_plugin_init(payload: OnPluginInitPayload<Self>) -> OnPluginInitResult<Self> {
    ///     if let Err(e) = do_some_initialization() {
    ///         return payload.error(e);
    ///     }
    ///     // continue with initialization...
    /// }
    /// ```
    pub fn error<TError>(err: TError) -> OnPluginInitResult<TRouterPlugin>
    where
        TError: Error + Into<BoxError>,
    {
        Err(err.into())
    }
}
