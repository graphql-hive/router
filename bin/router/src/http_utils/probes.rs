use std::sync::Arc;

use hive_router_plan_executor::hooks::on_supergraph_load::SupergraphSnapshot;
use ntex::web::{self, HttpRequest, Responder};

use crate::schema_state::SchemaState;

pub async fn health_check_handler() -> impl Responder {
    web::HttpResponse::Ok()
}

pub async fn readiness_check_handler(
    req: HttpRequest,
    schema_state: web::types::State<Arc<SchemaState>>,
) -> impl Responder {
    let plugin_selected = req.extensions().get::<SupergraphSnapshot>().is_some();
    // TODO: what if the plugin selected supergraph does not build? basically, plugins service sets
    //       `SupergraphSnapshot` on the request extensions once plugin hooks provide their supergraph.
    //       however, this snapshot is a pure schema-only snapshot. the selected supergraph, on the other
    //       hand is build for the `SchemaState` and that build can fail - so readiness should check if
    //       this build is going to pass in order to be accurate with the 200 OK response
    if plugin_selected || schema_state.is_ready() {
        web::HttpResponse::Ok()
    } else {
        web::HttpResponse::ServiceUnavailable()
    }
}
