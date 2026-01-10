use futures::future::ready;
use ntex::service::{fn_factory_with_config, fn_service, Service};
use ntex::web::{self, ws, Error, HttpRequest, HttpResponse};
use std::io;
use std::sync::Arc;

use crate::schema_state::SchemaState;
use crate::shared_state::RouterSharedState;

pub async fn graphql_ws_handler(
    req: HttpRequest,
    schema_state: web::types::State<Arc<SchemaState>>,
    app_state: web::types::State<Arc<RouterSharedState>>,
) -> Result<HttpResponse, Error> {
    let schema_state = schema_state.get_ref().clone();
    let app_state = app_state.get_ref().clone();

    ws::start(
        req,
        fn_factory_with_config(move |sink: ws::WsSink| {
            let schema_state = schema_state.clone();
            let app_state = app_state.clone();
            async move { ws_service(sink, schema_state, app_state).await }
        }),
    )
    .await
}

async fn ws_service(
    _sink: ws::WsSink,
    _schema_state: Arc<SchemaState>,
    _app_state: Arc<RouterSharedState>,
) -> Result<impl Service<ws::Frame, Response = Option<ws::Message>, Error = io::Error>, web::Error>
{
    let service = fn_service(move |frame| {
        let item = match frame {
            ws::Frame::Text(text) => Some(ws::Message::Text(
                String::from_utf8(Vec::from(text.as_ref())).unwrap().into(),
            )),
            ws::Frame::Binary(bin) => Some(ws::Message::Binary(bin)),
            ws::Frame::Close(reason) => Some(ws::Message::Close(reason)),
            ws::Frame::Ping(msg) => Some(ws::Message::Pong(msg)),
            ws::Frame::Pong(_) => None,
            _ => None,
        };
        ready(Ok(item))
    });

    Ok(service)
}
