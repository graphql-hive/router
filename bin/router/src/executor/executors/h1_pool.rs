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
//! A connection has no task of its own. hyper's connection future sits in the pool next to its
//! sender, and the request that holds it polls it while it waits for the connection to be ready,
//! for the response and for the body. So a request and its socket I/O run in one task: no
//! handoff to a connection task and back for every request, and nothing to wake on the side.
//! The catch is that nobody polls an idle connection, so one the subgraph closed is only noticed
//! at the next checkout, which drops it and takes another (or sends again, see `request`).
//!
//! Every connection also keeps its read buffer for good, and hyper grows it up to ~400KB while
//! reading big responses. It's capped by `traffic_shaping.all.http1_max_read_buffer_size`
//! (64KiB by default) like hyper's client, which takes a few more reads for a big response but
//! costs almost nothing.

use std::{
    future::{poll_fn, Future},
    pin::{pin, Pin},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    task::{Context, Poll, Wake, Waker},
    time::{Duration, Instant},
};

use ahash::AHashMap;
use bytes::Bytes;
use futures::task::AtomicWaker;
use http::{header::HOST, uri::Authority, HeaderValue, Request, Response, Uri};
use http_body_util::Full;
use hyper::{
    body::Incoming,
    client::conn::http1::{self, Connection, SendRequest},
};
use hyper_util::rt::TokioIo;
use tokio::net::TcpStream;

use crate::executor::executors::error::SubgraphExecutorError;

type Sender = SendRequest<Full<Bytes>>;

/// How a pool keeps its connections
#[derive(Clone, Copy)]
pub struct H1PoolConfig {
    pub idle_timeout: Duration,
    pub max_read_buffer_size: usize,
}
/// hyper's side of the socket. Boxed once per connection so it can move in and out of the pool.
type Conn = Pin<Box<Connection<TokioIo<TcpStream>, Full<Bytes>>>>;

/// An idle connection: the sender, the connection future nobody polls right now, and since when.
struct Idle {
    sender: Sender,
    conn: Conn,
    wakers: Arc<Wakers>,
    since: Instant,
}

/// One side of [`drive`]: the request or the connection. Its waker notes that this side has
/// something to do and wakes the task, so `drive` polls only the side that was woken.
struct Side {
    woken: AtomicBool,
    task: Arc<AtomicWaker>,
}

impl Wake for Side {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.woken.store(true, Ordering::Release);
        self.task.wake();
    }
}

/// The wakers `drive` polls the two sides with. Made once per connection, so a request doesn't
/// allocate any.
struct Wakers {
    task: Arc<AtomicWaker>,
    request: Arc<Side>,
    request_waker: Waker,
    conn: Arc<Side>,
    conn_waker: Waker,
}

impl Wakers {
    fn new() -> Arc<Self> {
        let task = Arc::new(AtomicWaker::new());
        let side = || {
            Arc::new(Side {
                woken: AtomicBool::new(true),
                task: Arc::clone(&task),
            })
        };
        let (request, conn) = (side(), side());
        Arc::new(Wakers {
            request_waker: Waker::from(Arc::clone(&request)),
            conn_waker: Waker::from(Arc::clone(&conn)),
            task,
            request,
            conn,
        })
    }
}

pub struct H1Pool {
    config: H1PoolConfig,
    /// Idle connections per origin, the most recently used last. A per-worker pool only takes
    /// the lock from its own thread, the lock is there for the shared one and because request
    /// futures have to be `Send`.
    idle: Mutex<AHashMap<Authority, Vec<Idle>>>,
}

/// A connection that is serving one request.
pub struct Lease<'a> {
    pool: &'a H1Pool,
    key: Authority,
    sender: Sender,
    /// `None` once the connection is done (closed by the subgraph, or failed)
    conn: Option<Conn>,
    wakers: Arc<Wakers>,
}

impl Lease<'_> {
    /// Runs `fut`, the read of the response body, while polling the connection that feeds it.
    pub async fn drive<F: Future>(&mut self, fut: F) -> F::Output {
        drive(&mut self.conn, &self.wakers, fut).await
    }

    /// Puts the connection back. Only call it once the whole response body was read.
    /// Dropping the lease instead closes the connection, and so does a connection that's done.
    pub fn release(self) {
        let Some(conn) = self.conn else {
            return;
        };
        let mut idle = self.pool.idle.lock().unwrap();
        let list = idle.entry(self.key).or_default();
        evict_expired(list, self.pool.config.idle_timeout);
        list.push(Idle {
            sender: self.sender,
            conn,
            wakers: self.wakers,
            since: Instant::now(),
        });
    }
}

impl H1Pool {
    pub fn new(config: H1PoolConfig) -> Self {
        Self {
            config,
            idle: Mutex::new(AHashMap::new()),
        }
    }

    /// Sends `req` and returns the response head. Read its body with [`Lease::drive`], the
    /// connection only makes progress while the lease's holder polls it.
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
            let (mut sender, mut conn, wakers, reused) = self.checkout(&key).await?;
            match drive(&mut conn, &wakers, sender.try_send_request(req)).await {
                Ok(response) => {
                    return Ok((
                        response,
                        Lease {
                            pool: self,
                            key,
                            sender,
                            conn,
                            wakers,
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
    async fn checkout(
        &self,
        key: &Authority,
    ) -> Result<(Sender, Option<Conn>, Arc<Wakers>, bool), SubgraphExecutorError> {
        while let Some(Idle {
            mut sender,
            conn,
            wakers,
            ..
        }) = self.pop(key)
        {
            // Polling the connection lets it finish up the last response and take the next
            // request. If the subgraph closed it meanwhile, it ends here and `ready` fails.
            let mut conn = Some(conn);
            if drive(&mut conn, &wakers, sender.ready()).await.is_ok() {
                return Ok((sender, conn, wakers, true));
            }
        }
        let (sender, conn) = connect(key, self.config.max_read_buffer_size).await?;
        Ok((sender, Some(conn), Wakers::new(), false))
    }

    fn pop(&self, key: &Authority) -> Option<Idle> {
        let mut idle = self.idle.lock().unwrap();
        let list = idle.get_mut(key)?;
        evict_expired(list, self.config.idle_timeout);
        list.pop()
    }
}

/// Polls `fut` and the connection together until `fut` is done. A connection that finishes
/// meanwhile (the subgraph closed it, or it failed) is dropped, and whatever waits on it then
/// fails on its own: hyper ends the sender's side of a finished connection.
///
/// Each side gets its own waker, and a wakeup only polls the side it came from. Most wakeups
/// are for one of them: the socket is readable (the connection), or the connection handed the
/// request its response (the request).
async fn drive<F: Future>(conn: &mut Option<Conn>, wakers: &Wakers, fut: F) -> F::Output {
    let mut fut = pin!(fut);
    // a new future hasn't been polled yet, and the connection may have work left from the
    // previous step
    wakers.request.woken.store(true, Ordering::Relaxed);
    wakers.conn.woken.store(true, Ordering::Relaxed);
    poll_fn(|cx| {
        wakers.task.register(cx.waker());
        // a side woken while we poll has also woken the task, so stopping early loses nothing,
        // the cap only keeps a future that keeps waking itself from spinning here
        for _ in 0..4 {
            // the request goes first, so a request it just queued gets written in this same poll
            if wakers.request.woken.swap(false, Ordering::AcqRel) {
                let mut request_cx = Context::from_waker(&wakers.request_waker);
                if let Poll::Ready(output) = fut.as_mut().poll(&mut request_cx) {
                    return Poll::Ready(output);
                }
            }
            // once the connection is gone, only the request's own wakeups are left
            if let Some(c) = conn.as_mut() {
                if wakers.conn.woken.swap(false, Ordering::AcqRel) {
                    let mut conn_cx = Context::from_waker(&wakers.conn_waker);
                    if c.as_mut().poll(&mut conn_cx).is_ready() {
                        *conn = None;
                        // what waits on it is told by hyper, but poll it to see that now
                        wakers.request.woken.store(true, Ordering::Relaxed);
                    }
                }
            }
            // the connection usually hands the request its response while we polled it: go
            // again now, not after another trip through the scheduler
            if !wakers.request.woken.load(Ordering::Acquire) {
                return Poll::Pending;
            }
        }
        // the request still has work, come back after others had a turn
        cx.waker().wake_by_ref();
        Poll::Pending
    })
    .await
}

// ponytail: expired connections are only dropped when the pool touches the origin again,
// so a pool that goes quiet keeps its idle sockets. Add a timer if that shows up.
fn evict_expired(list: &mut Vec<Idle>, idle_timeout: Duration) {
    let now = Instant::now();
    let expired = list.partition_point(|idle| now - idle.since >= idle_timeout);
    list.drain(..expired);
}

async fn connect(
    key: &Authority,
    max_read_buffer_size: usize,
) -> Result<(Sender, Conn), SubgraphExecutorError> {
    // `[::1]` comes with brackets, the resolver wants them gone
    let host = key.host().trim_start_matches('[').trim_end_matches(']');
    let tcp = TcpStream::connect((host, key.port_u16().unwrap_or(80)))
        .await
        .map_err(SubgraphExecutorError::ConnectFailure)?;
    tcp.set_nodelay(true)
        .map_err(SubgraphExecutorError::ConnectFailure)?;
    let (sender, connection) = http1::Builder::new()
        .max_buf_size(max_read_buffer_size)
        .handshake(TokioIo::new(tcp))
        .await?;
    Ok((sender, Box::pin(connection)))
}
