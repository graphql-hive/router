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
            .register::<subgraph_response_cache_plugin_example::SubgraphResponseCachePlugin>(),
    )
    .await
}
