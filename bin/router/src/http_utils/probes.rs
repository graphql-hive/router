use std::sync::Arc;

use ntex::web::{self, HttpRequest, Responder};

use crate::http_utils::app_state::AppData;
use crate::schema_state::SchemaState;

pub async fn health_check_handler() -> impl Responder {
    web::HttpResponse::Ok()
}

pub async fn readiness_check_handler(
    req: HttpRequest,
    schema_state: AppData<Arc<SchemaState>>,
) -> impl Responder {
    if schema_state.is_ready(&req).await {
        web::HttpResponse::Ok()
    } else {
        web::HttpResponse::ServiceUnavailable()
    }
}
