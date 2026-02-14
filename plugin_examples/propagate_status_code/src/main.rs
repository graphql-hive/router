use hive_router::{
    error::RouterInitError, init_rustls_crypto_provider, ntex, router_entrypoint,
    DefaultGlobalAllocator, PluginRegistry,
};

#[global_allocator]
static GLOBAL: DefaultGlobalAllocator = DefaultGlobalAllocator;

#[hive_router::main]
async fn main() -> Result<(), RouterInitError> {
    init_rustls_crypto_provider();

    router_entrypoint(
        PluginRegistry::new()
            .register::<propagate_status_code_plugin_example::PropagateStatusCodePlugin>(),
    )
    .await
}
