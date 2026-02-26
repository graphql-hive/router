![Hive GraphQL Platform](https://the-guild.dev/graphql/hive/github-org-image.png)

# [`hive-router`](https://crates.io/crates/hive-router)

A fully open-source MIT-licensed GraphQL API router that can act as a [GraphQL federation](https://the-guild.dev/graphql/hive/federation) Router, built with Rust for maximum performance and robustness.

This crate helps you create a custom build of the Hive Router with your own plugins.

```rust
use hive_router::{
    configure_global_allocator, error::RouterInitError, init_rustls_crypto_provider, ntex,
    router_entrypoint, PluginRegistry, RouterGlobalAllocator,
};
use hive_router::plugins::plugin_trait::RouterPlugin;
 
// Configure the global allocator required for Hive Router runtime
configure_global_allocator!();
 
// Declare and implement a simple plugin with no configuration and no hooks.
struct MyPlugin;
 
#[async_trait]
impl RouterPlugin for MyPlugin {
    // You can override this and add a custom config to your plugin
    type Config = ();
 
    fn plugin_name() -> &'static str {
        "my_plugin"
    }
 
    // Your hooks implementation goes here...
}
 
// This is the main entrypoint of the Router
#[hive_router::main]
async fn main() -> Result<(), RouterInitError> {
    // Configure Hive Router to use the OS's default certificate store
    init_rustls_crypto_provider();
 
    // Start and run the router entrypoint with your plugin
    router_entrypoint(
        PluginRegistry::new().register::<MyPlugin>(),
    )
    .await
}
```

- To learn more about the router itself, see the [router documentation](https://graphql-hive.com/docs/router).
- To learn how to extend the router, see the [extensibility guide](https://graphql-hive.com/docs/router/extensibility/plugin_system).
- To learn more about the plugin system API, see the [API reference](https://graphql-hive.com/docs/router/extensibility/plugin_system).
- If you don't need to extend the router, refer to the [getting started guide](https://graphql-hive.com/docs/router/getting-started).
