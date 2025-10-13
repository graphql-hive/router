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
        PluginRegistry::new().register::<usage_reporting_plugin_example::UsageReportingPlugin>(),
    )
    .await
}
