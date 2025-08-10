use std::env;

use mimalloc::MiMalloc;
use ntex::web::{self};
use query_planner::utils::parsing::parse_schema;

use crate::{
    http_utils::landing_page::landing_page_handler,
    logger::{configure_logging, LoggingFormat},
    pipeline::graphql_request_handler,
    shared_state::GatewaySharedState,
};

mod http_utils;
mod logger;
mod pipeline;
mod shared_state;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

#[ntex::main]
async fn main() -> std::io::Result<()> {
    let log_format = env::var("LOG_FORMAT")
        .map(|v| match v.as_str().to_lowercase() {
            str if str == "json" => LoggingFormat::Json,
            str if str == "tree" => LoggingFormat::PrettyTree,
            str if str == "compact" => LoggingFormat::PrettyCompact,
            _ => LoggingFormat::PrettyCompact,
        })
        .unwrap_or(LoggingFormat::PrettyCompact);
    configure_logging(log_format);

    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: gateway <SUPERGRAPH_PATH>");
        panic!("Missing supergraph path argument");
    }

    let supergraph_path = &args[1];
    let supergraph_sdl =
        std::fs::read_to_string(supergraph_path).expect("Unable to read input file");
    let parsed_schema = parse_schema(&supergraph_sdl);
    let gateway_shared_state = GatewaySharedState::new(parsed_schema);

    web::HttpServer::new(move || {
        web::App::new()
            .state(gateway_shared_state.clone())
            .route("/graphql", web::to(graphql_request_handler))
            .default_service(web::to(landing_page_handler))
    })
    .bind(("0.0.0.0", 4000))?
    .run()
    .await
}
