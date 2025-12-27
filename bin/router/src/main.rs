use hive_router::router_entrypoint;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[ntex::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    match router_entrypoint(None).await {
        Ok(_) => Ok(()),
        Err(err) => {
            eprintln!("Failed to start Hive Router:\n  {}", err);

            Err(err)
        }
    }
}
