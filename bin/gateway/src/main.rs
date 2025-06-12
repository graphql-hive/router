mod handlers;

use axum::{
    extract::State,
    http::{HeaderMap, Method},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use handlers::{
    graphiql_handler, graphql_get_handler, graphql_post_handler, landing_page_handler,
    GraphQLQueryParams,
};
use query_plan_executor::schema_metadata::{SchemaMetadata, SchemaWithMetadata};
use query_planner::planner::Planner;
use query_planner::state::supergraph_state::SupergraphState;
use query_planner::utils::parsing::parse_schema;
use std::{collections::HashMap, env, net::SocketAddr, sync::Arc};
use tokio::net::TcpListener;
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

struct AppState {
    supergraph_source: String,
    schema_metadata: SchemaMetadata,
    planner: Planner,
    validation_plan: graphql_tools::validation::validate::ValidationPlan,
    subgraph_endpoint_map: HashMap<String, String>,
    http_client: reqwest::Client,
    plan_cache: moka::future::Cache<u64, Arc<query_planner::planner::plan_nodes::QueryPlan>>,
    validate_cache:
        moka::future::Cache<u64, Arc<Vec<graphql_tools::validation::utils::ValidationError>>>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let tree_layer = tracing_tree::HierarchicalLayer::new(2)
        .with_bracketed_fields(true)
        .with_deferred_spans(false)
        .with_wraparound(25)
        .with_indent_lines(true)
        .with_timer(tracing_tree::time::Uptime::default())
        .with_thread_names(false)
        .with_thread_ids(false)
        .with_targets(false);

    tracing_subscriber::registry()
        .with(tree_layer)
        .with(EnvFilter::from_default_env())
        .init();

    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: gateway <SUPERGRAPH_PATH>");
        return Err("Missing supergraph path argument".into());
    }

    let supergraph_path = &args[1];

    let supergraph_sdl =
        std::fs::read_to_string(supergraph_path).expect("Unable to read input file");
    let parsed_schema = parse_schema(&supergraph_sdl);
    let supergraph_state = SupergraphState::new(&parsed_schema);
    let planner = Planner::new_from_supergraph(&parsed_schema).expect("failed to create planner");
    let schema_metadata = planner.consumer_schema.schema_metadata();

    let app_state = Arc::new(AppState {
        supergraph_source: supergraph_path.to_string(),
        schema_metadata: schema_metadata,
        planner,
        validation_plan: graphql_tools::validation::rules::default_rules_validation_plan(),
        subgraph_endpoint_map: supergraph_state.subgraph_endpoint_map,
        http_client: reqwest::Client::new(),
        plan_cache: moka::future::Cache::new(1000),
        validate_cache: moka::future::Cache::new(1000),
    });

    async fn universal_graphql_get_handler(
        State(app_state): State<Arc<AppState>>,
        headers: HeaderMap,
        params: axum::extract::Query<GraphQLQueryParams>,
    ) -> Response {
        let accept_header = headers
            .get(axum::http::header::ACCEPT)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");

        if accept_header.contains("text/html") {
            graphiql_handler().await.into_response()
        } else {
            graphql_get_handler(State(app_state), headers, params)
                .await
                .into_response()
        }
    }

    let app = Router::new()
        .route(
            "/graphql",
            get(universal_graphql_get_handler).post(graphql_post_handler),
        )
        .fallback(get(landing_page_handler))
        .with_state(app_state)
        .layer(
            CorsLayer::new()
                .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
                .allow_headers(vec![
                    axum::http::header::ACCEPT,
                    axum::http::header::CONTENT_TYPE,
                ])
                .allow_origin(tower_http::cors::Any),
        )
        .layer(TraceLayer::new_for_http());

    let addr = SocketAddr::from(([0, 0, 0, 0], 4000));
    info!("Starting server on {}", addr);

    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, app.into_make_service()).await?;

    Ok(())
}
