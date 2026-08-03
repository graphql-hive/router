// Needed because of ntex's way of defining middlewares
#![recursion_limit = "256"]

use hive_router::{
    configure_global_allocator, error::RouterInitError, init_rustls_crypto_provider, ntex,
    router_entrypoint, PluginRegistry, RouterGlobalAllocator,
};

configure_global_allocator!();

#[hive_router::main]
async fn main() -> Result<(), RouterInitError> {
    init_rustls_crypto_provider();

    router_entrypoint(PluginRegistry::new().register::<plugin_template::plugin::MyPlugin>()).await
}
