use ntex::web::{self, HttpRequest, Responder};

use crate::RouterAppState;

// responders are typed by the app state of the app serving them
pub async fn health_check_handler() -> impl Responder<RouterAppState> {
    web::HttpResponse::Ok()
}

pub async fn readiness_check_handler(
    state: &RouterAppState,
    _: (),
    req: HttpRequest,
) -> impl Responder<RouterAppState> {
    if state.schema.is_ready(&req).await {
        web::HttpResponse::Ok()
    } else {
        web::HttpResponse::ServiceUnavailable()
    }
}
