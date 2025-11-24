use hive_router::{PluginRegistry, router_entrypoint};
use hive_router_plan_executor::examples::apq::APQPlugin;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[ntex::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut plugin_factories = PluginRegistry::new();
    plugin_factories.register::<APQPlugin>();

    match router_entrypoint(plugin_factories).await {
        Ok(_) => Ok(()),
        Err(err) => {
            eprintln!("Failed to start Hive Router:\n  {}", err);

            Err(err)
        }
    }
}
