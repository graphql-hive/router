//! A strict HTTP/1 connection pool for plain `http://` subgraphs.
//! On with `traffic_shaping.experimental_http1_pool`, one for all workers or one per worker,
//! following `traffic_shaping.http_client_mode`.
//!
//! hyper's legacy pool races a fresh connect against every checkout that finds no idle
//! connection, and keeps the new connection even when an idle one came back first. A finished
//! connection only goes back to its pool from a task spawned after the response, so under load
//! checkouts miss all the time and the pool grows to thousands of sockets. Here a connection
//! goes back to the idle list right after its response body is read, so a request only
//! connects when every connection of its pool is busy.
//!
//! Every connection also keeps its read buffer for good, and hyper grows it up to ~400KB while
//! reading big responses. We cap it at 64KB, which takes a few more reads for a big response
//! but costs nothing measurable, even for 3MB bodies.

use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

use ahash::AHashMap;
use bytes::Bytes;
use http::{header::HOST, uri::Authority, HeaderValue, Request, Response, Uri};
use http_body_util::Full;
use hyper::{
    body::Incoming,
    client::conn::http1::{self, SendRequest},
};
use hyper_util::rt::TokioIo;
use tokio::net::TcpStream;

use crate::executor::executors::error::SubgraphExecutorError;

const MAX_BUF_SIZE: usize = 64 * 1024;

type Sender = SendRequest<Full<Bytes>>;

pub struct H1Pool {
    idle_timeout: Duration,
    /// Idle connections per origin, the most recently used last. A per-worker pool only takes
    /// the lock from its own thread, the lock is there for the shared one and because request
    /// futures have to be `Send`.
    idle: Mutex<AHashMap<Authority, Vec<(Sender, Instant)>>>,
}

/// A connection that is serving one request.
pub struct Lease<'a> {
    pool: &'a H1Pool,
    key: Authority,
    sender: Sender,
}

impl Lease<'_> {
    /// Puts the connection back. Only call it once the whole response body was read.
    /// Dropping the lease instead closes the connection.
    pub fn release(self) {
        let mut idle = self.pool.idle.lock().unwrap();
        let list = idle.entry(self.key).or_default();
        evict_expired(list, self.pool.idle_timeout);
        list.push((self.sender, Instant::now()));
    }
}

impl H1Pool {
    pub fn new(idle_timeout: Duration) -> Self {
        Self {
            idle_timeout,
            idle: Mutex::new(AHashMap::new()),
        }
    }

    pub async fn request(
        &self,
        mut req: Request<Full<Bytes>>,
    ) -> Result<(Response<Incoming>, Lease<'_>), SubgraphExecutorError> {
        let uri = req.uri();
        let Some(key) = uri.authority().cloned() else {
            return Err(SubgraphExecutorError::ConnectFailure(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "subgraph endpoint has no host",
            )));
        };
        // same as hyper's client: origin-form target, and a Host header without the default port
        let origin_form = match uri.path_and_query() {
            Some(path) => Uri::builder().path_and_query(path.clone()).build()?,
            None => Uri::from_static("/"),
        };
        *req.uri_mut() = origin_form;
        if !req.headers().contains_key(HOST) {
            let host = match key.port_u16() {
                Some(80) | None => HeaderValue::from_str(key.host()),
                Some(_) => HeaderValue::from_str(key.as_str()),
            };
            if let Ok(host) = host {
                req.headers_mut().insert(HOST, host);
            }
        }

        loop {
            let (mut sender, reused) = self.checkout(&key).await?;
            match sender.try_send_request(req).await {
                Ok(response) => {
                    return Ok((
                        response,
                        Lease {
                            pool: self,
                            key,
                            sender,
                        },
                    ))
                }
                // an idle connection the subgraph closed before the request went out: sent
                // again on another one, like hyper's client does
                Err(mut err) if reused => match err.take_message() {
                    Some(back) => req = back,
                    None => return Err(err.into_error().into()),
                },
                Err(err) => return Err(err.into_error().into()),
            }
        }
    }

    /// A connection for `key`, and whether it was an idle one
    async fn checkout(&self, key: &Authority) -> Result<(Sender, bool), SubgraphExecutorError> {
        while let Some(mut sender) = self.pop(key) {
            // The last response on it was read already, so this resolves as soon as the
            // connection task catches up. An error means the subgraph closed it meanwhile.
            if sender.ready().await.is_ok() {
                return Ok((sender, true));
            }
        }
        Ok((connect(key).await?, false))
    }

    fn pop(&self, key: &Authority) -> Option<Sender> {
        let mut idle = self.idle.lock().unwrap();
        let list = idle.get_mut(key)?;
        evict_expired(list, self.idle_timeout);
        list.pop().map(|(sender, _)| sender)
    }
}

// ponytail: expired connections are only dropped when the pool touches the origin again,
// so a pool that goes quiet keeps its idle sockets. Add a timer if that shows up.
fn evict_expired(list: &mut Vec<(Sender, Instant)>, idle_timeout: Duration) {
    let now = Instant::now();
    let expired = list.partition_point(|(_, since)| now - *since >= idle_timeout);
    list.drain(..expired);
}

async fn connect(key: &Authority) -> Result<Sender, SubgraphExecutorError> {
    // `[::1]` comes with brackets, the resolver wants them gone
    let host = key.host().trim_start_matches('[').trim_end_matches(']');
    let tcp = TcpStream::connect((host, key.port_u16().unwrap_or(80)))
        .await
        .map_err(SubgraphExecutorError::ConnectFailure)?;
    tcp.set_nodelay(true)
        .map_err(SubgraphExecutorError::ConnectFailure)?;
    let (sender, connection) = http1::Builder::new()
        .max_buf_size(MAX_BUF_SIZE)
        .handshake(TokioIo::new(tcp))
        .await?;
    tokio::spawn(async move {
        let _ = connection.await;
    });
    Ok(sender)
}
