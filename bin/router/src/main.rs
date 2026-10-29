// Needed because of ntex's way of defining middlewares
#![recursion_limit = "256"]

use hive_router::{
    error::RouterInitError, init_rustls_crypto_provider, router_entrypoint, PluginRegistry,
};

#[cfg(not(feature = "dhat-heap"))]
use hive_router::{configure_global_allocator, RouterGlobalAllocator};
#[cfg(not(feature = "dhat-heap"))]
configure_global_allocator!();

#[cfg(feature = "dhat-heap")]
#[global_allocator]
static GLOBAL: dhat::Alloc = dhat::Alloc;

#[hive_router::main]
async fn main() -> Result<(), RouterInitError> {
    // writes dhat-heap.json to cwd when dropped, so the router has to shut down cleanly (SIGINT/SIGTERM)
    #[cfg(feature = "dhat-heap")]
    // full backtraces, the default cuts them at 10 frames and recursive Value clones need more
    let _profiler = dhat::Profiler::builder().trim_backtraces(None).build();

    // `args_os`, because `args` panics on an argument that isn't valid unicode, and the router
    // itself never reads argv
    if std::env::args_os()
        .nth(1)
        .is_some_and(|command| command == "anonymize")
    {
        let args = std::env::args_os()
            .skip(2)
            .map(|arg| arg.into_string())
            .collect::<Result<Vec<_>, _>>();
        let result = match args {
            Ok(args) => hive_router::anonymize::run_cli(&args),
            Err(arg) => Err(format!("{arg:?} isn't valid unicode")),
        };
        if let Err(err) = result {
            eprintln!("{err}");
            std::process::exit(1);
        }
        return Ok(());
    }

    init_rustls_crypto_provider();

    router_entrypoint(PluginRegistry::new()).await
}
