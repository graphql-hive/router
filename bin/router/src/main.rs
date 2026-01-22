use hive_router::{error::RouterInitError, init_rustls_crypto_provider, router_entrypoint};

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[ntex::main]
async fn main() -> Result<(), Box<RouterInitError>> {
    init_rustls_crypto_provider();

    match router_entrypoint().await {
        Ok(_) => Ok(()),
        Err(err) => {
            eprintln!("Failed to start Hive Router:\n  {}", err);

            Err(err.into())
        }
    }
}
