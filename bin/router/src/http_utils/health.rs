use ntex::web::{self, Responder};

pub async fn health_check_handler() -> impl Responder {
    web::HttpResponse::Ok()
}
