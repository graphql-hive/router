pub mod background_tasks;
mod consts;
mod http_utils;
mod jwt;
mod logger;
pub mod pipeline;
mod schema_state;
mod shared_state;
mod supergraph;
mod utils;

use std::sync::Arc;

use crate::{
    background_tasks::BackgroundTasksManager,
    consts::ROUTER_VERSION,
    http_utils::{
        landing_page::landing_page_handler,
        probes::{health_check_handler, readiness_check_handler},
    },
    jwt::JwtAuthRuntime,
    logger::configure_logging,
    pipeline::{
        graphql_request_handler,
        header::{RequestAccepts, TEXT_HTML_MIME},
        usage_reporting::init_hive_user_agent,
    },
};

pub use crate::{schema_state::SchemaState, shared_state::RouterSharedState};

use hive_router_config::{load_config, HiveRouterConfig};
use http::{
    header::{CONTENT_TYPE, RETRY_AFTER},
    Method,
};
use ntex::{
    util::Bytes,
    web::{self, HttpRequest},
};
use tracing::{info, warn};

static GRAPHIQL_HTML: &str = include_str!("../static/graphiql.html");

async fn graphql_endpoint_handler(
    request: HttpRequest,
    body_bytes: Bytes,
    schema_state: web::types::State<Arc<SchemaState>>,
    app_state: web::types::State<Arc<RouterSharedState>>,
) -> impl web::Responder {
    let maybe_supergraph = schema_state.current_supergraph();

    if let Some(supergraph) = maybe_supergraph.as_ref() {
        // If an early CORS response is needed, return it immediately.
        if let Some(early_response) = app_state
            .cors_runtime
            .as_ref()
            .and_then(|cors| cors.get_early_response(&request))
        {
            return early_response;
        }

        // Aggree on the response content type so that errors can be handled
        // properly outside the request handler.
        let (single_content_type, stream_content_type) = match request.negotiate() {
            Ok((single, stream)) => (single, stream),
            Err(err) => return err.into_response(None),
        };

        if request.method() == Method::GET
        && single_content_type.is_none()
        // coming soon
        // && stream_content_type.is_none()
        && request.can_accept_http()
        {
            if app_state.router_config.graphiql.enabled {
                return web::HttpResponse::Ok()
                    .header(CONTENT_TYPE, TEXT_HTML_MIME)
                    .body(GRAPHIQL_HTML);
            } else {
                return web::HttpResponse::NotFound().into();
            }
        }

        let mut res = match graphql_request_handler(
            &request,
            body_bytes,
            single_content_type.clone(),
            stream_content_type,
            supergraph,
            app_state.get_ref(),
            schema_state.get_ref(),
        )
        .await
        {
            Ok(response) => response,
            Err(err) => return err.into_response(single_content_type),
        };

        // Apply CORS headers to the final response if CORS is configured.
        if let Some(cors) = app_state.cors_runtime.as_ref() {
            cors.set_headers(&request, res.headers_mut());
        }

        res
    } else {
        warn!("No supergraph available yet, unable to process request");

        web::HttpResponse::ServiceUnavailable()
            .header(RETRY_AFTER, 10)
            .finish()
    }
}

pub async fn router_entrypoint() -> Result<(), Box<dyn std::error::Error>> {
    let config_path = std::env::var("ROUTER_CONFIG_FILE_PATH").ok();
    let router_config = load_config(config_path)?;
    configure_logging(&router_config.log);
    info!("hive-router@{} starting...", ROUTER_VERSION);
    let http_config = router_config.http.clone();
    let addr = router_config.http.address();
    let mut bg_tasks_manager = BackgroundTasksManager::new();
    let (shared_state, schema_state) =
        configure_app_from_config(router_config, &mut bg_tasks_manager).await?;

    let maybe_error = web::HttpServer::new(move || {
        let lp_gql_path = http_config.graphql_endpoint().to_string();
        web::App::new()
            .state(shared_state.clone())
            .state(schema_state.clone())
            .configure(|m| configure_ntex_app(m, http_config.graphql_endpoint()))
            .default_service(web::to(move || landing_page_handler(lp_gql_path.clone())))
    })
    .bind(addr)?
    .run()
    .await
    .map_err(|err| err.into());

    info!("server stopped, clearning background tasks");
    bg_tasks_manager.shutdown();

    maybe_error
}

pub async fn configure_app_from_config(
    router_config: HiveRouterConfig,
    bg_tasks_manager: &mut BackgroundTasksManager,
) -> Result<(Arc<RouterSharedState>, Arc<SchemaState>), Box<dyn std::error::Error>> {
    let jwt_runtime = match router_config.jwt.is_jwt_auth_enabled() {
        true => Some(JwtAuthRuntime::init(bg_tasks_manager, &router_config.jwt).await?),
        false => None,
    };

    let hive_usage_agent = match router_config.usage_reporting.enabled {
        true => Some(init_hive_user_agent(
            bg_tasks_manager,
            &router_config.usage_reporting,
        )?),
        false => None,
    };

    let router_config_arc = Arc::new(router_config);
    let schema_state =
        SchemaState::new_from_config(bg_tasks_manager, router_config_arc.clone()).await?;
    let schema_state_arc = Arc::new(schema_state);
    let shared_state = Arc::new(RouterSharedState::new(
        router_config_arc,
        jwt_runtime,
        hive_usage_agent,
    )?);

    Ok((shared_state, schema_state_arc))
}

pub fn configure_ntex_app(cfg: &mut web::ServiceConfig, graphql_path: &str) {
    cfg.route(graphql_path, web::to(graphql_endpoint_handler))
        .route("/health", web::to(health_check_handler))
        .route("/readiness", web::to(readiness_check_handler));
}
