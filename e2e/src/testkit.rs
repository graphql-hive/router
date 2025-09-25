use std::{path::PathBuf, sync::Once};

use hive_router::{
    background_tasks::BackgroundTasksManager, configure_app_from_config, configure_ntex_app,
};
use hive_router_config::load_config;
use lazy_static::lazy_static;
use ntex::{
    web::{self, test, test::TestRequest, WebResponse},
    Pipeline,
};
use sonic_rs::json;
use tracing::subscriber::set_global_default;
use tracing_subscriber::{fmt, layer::SubscriberExt, EnvFilter};

pub fn init_graphql_request(op: &str, variables: Option<sonic_rs::Value>) -> TestRequest {
    let body = json!({
      "query": op,
      "variables": variables
    });

    test::TestRequest::post()
        .uri("/graphql")
        .header("content-type", "application/json")
        .set_payload(body.to_string())
}

lazy_static! {
    static ref TRACING_INIT: Once = Once::new();
}

#[allow(dead_code)] // call this at the beginning of the test if you wish to see gw logs
pub fn init_logger() {
    TRACING_INIT.call_once(|| {
        let subscriber = tracing_subscriber::registry()
            .with(fmt::layer().with_test_writer())
            .with(EnvFilter::from_default_env());

        let _ = set_global_default(subscriber);
    });
}

pub async fn init_router_from_config_file(
    config_path: &str,
) -> Result<
    Pipeline<
        impl ntex::Service<ntex::http::Request, Response = WebResponse, Error = ntex::web::Error>,
    >,
    Box<dyn std::error::Error>,
> {
    // init_logger();

    let supergraph_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(config_path);
    let router_config = load_config(Some(supergraph_path.to_str().unwrap().to_string()))?;
    let mut bg_tasks_manager = BackgroundTasksManager::new();
    let shared_state = configure_app_from_config(router_config, &mut bg_tasks_manager).await?;

    let ntex_app = test::init_service(
        web::App::new()
            .state(shared_state.clone())
            .configure(configure_ntex_app),
    )
    .await;

    Ok(ntex_app)
}
