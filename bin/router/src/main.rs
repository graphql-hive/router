use hive_router::router_entrypoint;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[ntex::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    router_entrypoint().await
}
