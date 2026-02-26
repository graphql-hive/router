use hive_router::{
    configure_global_allocator, error::RouterInitError, init_rustls_crypto_provider, ntex,
    router_entrypoint, PluginRegistry, RouterGlobalAllocator,
};

configure_global_allocator!();

#[hive_router::main]
async fn main() -> Result<(), RouterInitError> {
    init_rustls_crypto_provider();

    router_entrypoint(
        PluginRegistry::new().register::<usage_reporting_plugin_example::UsageReportingPlugin>(),
    )
    .await
}
