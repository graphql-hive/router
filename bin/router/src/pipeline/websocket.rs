use futures::future::{ready, select, Either};
use ntex::channel::oneshot;
use ntex::service::{fn_factory_with_config, fn_service, fn_shutdown, Service};
use ntex::util::Bytes;
use ntex::web::{self, ws, Error, HttpRequest, HttpResponse};
use ntex::{chain, rt};
use std::cell::RefCell;
use std::io;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::debug;

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

struct WsState {
    /// The moment of the last heartbeat received from the client. This is used
    /// to detect client timeouts and drop the connection on timeout.
    last_heartbeat: Instant,
}

/// Heartbeat ping interval.
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);
/// Client response to heartbeat timeout.
const CLIENT_TIMEOUT: Duration = Duration::from_secs(10);
/// Ping client every heartbeat interval.
async fn heartbeat(
    state: Rc<RefCell<WsState>>,
    sink: web::ws::WsSink,
    mut rx: oneshot::Receiver<()>,
) {
    loop {
        match select(Box::pin(ntex::time::sleep(HEARTBEAT_INTERVAL)), &mut rx).await {
            Either::Left(_) => {
                if Instant::now().duration_since(state.borrow().last_heartbeat) > CLIENT_TIMEOUT {
                    debug!("WebSocket client heartbeat timeout");
                    return;
                }
                if sink
                    .send(web::ws::Message::Ping(Bytes::default()))
                    .await
                    .is_err()
                {
                    return;
                }
            }
            Either::Right(_) => return,
        }
    }
}

async fn ws_service(
    sink: ws::WsSink,
    _schema_state: Arc<SchemaState>,
    _app_state: Arc<RouterSharedState>,
) -> Result<impl Service<ws::Frame, Response = Option<ws::Message>, Error = io::Error>, web::Error>
{
    debug!("WebSocket connection established");

    let state = Rc::new(RefCell::new(WsState {
        last_heartbeat: Instant::now(),
    }));

    let (tx, rx) = oneshot::channel();

    rt::spawn(heartbeat(state.clone(), sink, rx));

    let service = fn_service(move |frame| {
        let item = match frame {
            ws::Frame::Text(text) => Some(ws::Message::Text(
                String::from_utf8(Vec::from(text.as_ref())).unwrap().into(),
            )),
            // we don't support binary frames
            // TODO: should we drop the connection?
            ws::Frame::Binary(_) => None,
            // heartbeat
            web::ws::Frame::Ping(msg) => {
                state.borrow_mut().last_heartbeat = Instant::now();
                Some(web::ws::Message::Pong(msg))
            }
            web::ws::Frame::Pong(_) => {
                state.borrow_mut().last_heartbeat = Instant::now();
                None
            }
            // client's closing
            ws::Frame::Close(reason) => Some(ws::Message::Close(reason)),
            // ignore other frames (should not match)
            _ => None,
        };
        ready(Ok(item))
    });

    let on_shutdown = fn_shutdown(move || {
        // stop heartbeat task on shutdown
        let _ = tx.send(());
    });

    Ok(chain(service).and_then(on_shutdown))
}
