use hive_router::router_entrypoint;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[ntex::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Uses aws as ring is no longer maintained.
    // The default provider is decided in the main function,
    // to let hive-router crate users pick their own crypto provider.
    // The rest of our crates, depends on rustls and relies on consumers
    // to define the crypto provider.
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .expect("installing default crypto provider");

    match router_entrypoint().await {
        Ok(_) => Ok(()),
        Err(err) => {
            eprintln!("Failed to start Hive Router:\n  {}", err);

            Err(err)
        }
    }
}
