use std::sync::Arc;

use ntex::web::{self, Responder};

use crate::schema_state::SchemaState;

pub async fn health_check_handler() -> impl Responder {
    web::HttpResponse::Ok()
}

pub async fn readiness_check_handler(
    schema_state: web::types::State<Arc<SchemaState>>,
) -> impl Responder {
    if schema_state.is_ready() {
        web::HttpResponse::Ok()
    } else {
        web::HttpResponse::InternalServerError()
    }
}
