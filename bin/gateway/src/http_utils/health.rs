use axum::http::StatusCode;
use axum::response::IntoResponse;

pub async fn health_check_handler() -> impl IntoResponse {
    StatusCode::OK
}
