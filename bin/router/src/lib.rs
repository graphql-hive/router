pub mod background_tasks;
mod consts;
pub mod error;
mod http_utils;
mod jwt;
pub mod pipeline;
mod schema_state;
mod shared_state;
mod supergraph;
pub mod telemetry;
mod utils;

use std::sync::Arc;

use crate::{
    background_tasks::BackgroundTasksManager,
    consts::ROUTER_VERSION,
    error::RouterInitError,
    http_utils::{
        landing_page::landing_page_handler,
        probes::{health_check_handler, readiness_check_handler},
    },
    jwt::JwtAuthRuntime,
    pipeline::{
        error::PipelineError,
        graphql_request_handler,
        header::{RequestAccepts, TEXT_HTML_MIME},
        usage_reporting::init_hive_usage_agent,
        validation::{max_depth_rule::MaxDepthRule, max_directives_rule::MaxDirectivesRule},
    },
    telemetry::HeaderExtractor,
};

pub use crate::{schema_state::SchemaState, shared_state::RouterSharedState};

use graphql_tools::validation::rules::default_rules_validation_plan;
use hive_router_config::{load_config, HiveRouterConfig};
use hive_router_internal::telemetry::{
    otel::tracing_opentelemetry::OpenTelemetrySpanExt,
    traces::spans::http_request::HttpServerRequestSpan, TelemetryContext,
};
use http::{
    header::{CONTENT_TYPE, RETRY_AFTER},
    Method,
};
use ntex::{
    util::Bytes,
    web::{self, HttpRequest},
};
use tracing::{info, warn, Instrument};

static GRAPHIQL_HTML: &str = include_str!("../static/graphiql.html");

async fn graphql_endpoint_handler(
    request: HttpRequest,
    body_bytes: Bytes,
    schema_state: web::types::State<Arc<SchemaState>>,
    app_state: web::types::State<Arc<RouterSharedState>>,
) -> impl web::Responder {
    // If an early CORS response is needed, return it immediately.
    if let Some(early_response) = app_state
        .cors_runtime
        .as_ref()
        .and_then(|cors| cors.get_early_response(&request))
    {
        return early_response;
    }

    // agree on the response content type so that errors can be handled
    // properly outside the request handler.
    let response_mode = match request.negotiate() {
        Ok(response_mode) => response_mode,
        Err(err) => return err.into_response(None),
    };

    if request.method() == Method::GET && response_mode.is_none() && request.can_accept_http() {
        if app_state.router_config.graphiql.enabled {
            return web::HttpResponse::Ok()
                .header(CONTENT_TYPE, TEXT_HTML_MIME)
                .body(GRAPHIQL_HTML);
        } else {
            return web::HttpResponse::NotFound().into();
        }
    }

    // not a graphiql request and no supported content types
    let response_mode = match response_mode {
        Some(mode) => mode,
        None => {
            return PipelineError::UnsupportedContentType.into_response(response_mode);
        }
    };

    let parent_ctx = app_state
        .telemetry_context
        .extract_context(&HeaderExtractor(request.headers()));
    let root_http_request_span = HttpServerRequestSpan::from_request(&request, &body_bytes);
    let _ = root_http_request_span.set_parent(parent_ctx);
    let span = root_http_request_span.span.clone();
    let maybe_supergraph = schema_state.current_supergraph();

    async {
        if let Some(supergraph) = maybe_supergraph.as_ref() {
            let mut res = match graphql_request_handler(
                &request,
                body_bytes,
                &response_mode,
                supergraph,
                app_state.get_ref(),
                schema_state.get_ref(),
            )
            .await
            {
                Ok(response) => response,
                Err(err) => return err.into_response(Some(response_mode)),
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
    .instrument(span)
    .await
}

pub async fn router_entrypoint() -> Result<(), RouterInitError> {
    let config_path = std::env::var("ROUTER_CONFIG_FILE_PATH").ok();
    let router_config = load_config(config_path)?;
    let telemetry = telemetry::Telemetry::init_global(&router_config);

    info!("hive-router@{} starting...", ROUTER_VERSION);
    let http_config = router_config.http.clone();
    let addr = router_config.http.address();
    let mut bg_tasks_manager = BackgroundTasksManager::new();
    let (shared_state, schema_state) = configure_app_from_config(
        router_config,
        telemetry.context.clone(),
        &mut bg_tasks_manager,
    )
    .await?;

    let maybe_error = web::HttpServer::new(move || {
        let lp_gql_path = http_config.graphql_endpoint().to_string();
        web::App::new()
            .state(shared_state.clone())
            .state(schema_state.clone())
            .configure(|m| configure_ntex_app(m, http_config.graphql_endpoint()))
            .default_service(web::to(move || landing_page_handler(lp_gql_path.clone())))
    })
    .bind(&addr)
    .map_err(|err| RouterInitError::HttpServerBindError(addr, err))?
    .run()
    .await
    .map_err(RouterInitError::HttpServerStartError);

    info!("server stopped, clearning background tasks");
    bg_tasks_manager.shutdown();
    telemetry.graceful_shutdown().await;

    maybe_error
}

pub async fn configure_app_from_config(
    router_config: HiveRouterConfig,
    telemetry_context: TelemetryContext,
    bg_tasks_manager: &mut BackgroundTasksManager,
) -> Result<(Arc<RouterSharedState>, Arc<SchemaState>), RouterInitError> {
    let jwt_runtime = match router_config.jwt.is_jwt_auth_enabled() {
        true => Some(JwtAuthRuntime::init(bg_tasks_manager, &router_config.jwt).await?),
        false => None,
    };

    let hive_usage_agent = match router_config.telemetry.hive.as_ref() {
        Some(hive_config) if hive_config.usage_reporting.enabled => {
            Some(init_hive_usage_agent(bg_tasks_manager, hive_config)?)
        }
        _ => None,
    };

    let router_config_arc = Arc::new(router_config);
    let telemetry_context_arc = Arc::new(telemetry_context);
    let schema_state = SchemaState::new_from_config(
        bg_tasks_manager,
        telemetry_context_arc.clone(),
        router_config_arc.clone(),
    )
    .await?;
    let schema_state_arc = Arc::new(schema_state);
    let mut validation_plan = default_rules_validation_plan();
    if let Some(max_depth_config) = &router_config_arc.limits.max_depth {
        validation_plan.add_rule(Box::new(MaxDepthRule {
            config: max_depth_config.clone(),
        }));
    }
    if let Some(max_directives_config) = &router_config_arc.limits.max_directives {
        validation_plan.add_rule(Box::new(MaxDirectivesRule {
            config: max_directives_config.clone(),
        }));
    }
    let shared_state = Arc::new(RouterSharedState::new(
        router_config_arc,
        jwt_runtime,
        hive_usage_agent,
        validation_plan,
        telemetry_context_arc,
    )?);

    Ok((shared_state, schema_state_arc))
}

pub fn configure_ntex_app(cfg: &mut web::ServiceConfig, graphql_path: &str) {
    cfg.route(graphql_path, web::to(graphql_endpoint_handler))
        .route("/health", web::to(health_check_handler))
        .route("/readiness", web::to(readiness_check_handler));
}

/// Initializes the rustls cryptographic provider for the entire process.
///
/// Rustls requires a cryptographic provider to be set as the default before any TLS operations occur.
/// Installs AWS-LC, as `ring` is no longer maintained.
///
/// This function should be called early in the application startup, before any rustls-based TLS
/// connections are established.
/// In the hive-router binary and docker image, it's called automatically during router initialization.
/// This ensures that all TLS operations throughout the application can use the configured provider.
///
/// This function can only be called successfully once per process.
/// Subsequent calls will log a warning, but will not fail.
///
///
/// This allows consumers of the `hive-router` crate to use their own cryptographic provider if needed,
/// by calling this function or setting their own provider before initializing the router.
///
/// This function does not return an error. If the provider is already installed, it logs a warning.
pub fn init_rustls_crypto_provider() {
    if rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .is_err()
    {
        warn!("Rustls crypto provider already installed");
    }
}
