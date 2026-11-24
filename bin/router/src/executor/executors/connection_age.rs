//! Retires subgraph connections that shouldn't be reused: ones past `max_connection_age`, and
//! ones whose request timed out (HTTP/1 closes those itself, HTTP/2 doesn't).
//!
//! The connector stamps each new connection with a deadline. A request that ran on a
//! connection past it poisons the connection when it ends, so the pool stops handing it out.

use std::{
    future::Future,
    io,
    pin::Pin,
    task::{Context, Poll},
    time::{Duration, Instant},
};

use bytes::Bytes;
use http::{Extensions, Request, Response, Uri};
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::rt::{Read, ReadBufCursor, Write};
use hyper_util::client::legacy::connect::{
    capture_connection, CaptureConnection, Connected, Connection, HttpConnector,
};
use hyper_util::rt::TokioIo;
use rand::prelude::*;
use tokio::net::TcpStream;
use tokio::time::error::Elapsed;
use tower_service::Service;

use super::http::HttpClient;

// hyper-util doesn't export the connector's error type by name.
type ConnectError = <HttpConnector as Service<Uri>>::Error;

/// When a connection should stop taking new requests. Stored in the connection's metadata.
#[derive(Clone, Copy, Debug)]
struct ConnectionDeadline(Instant);

/// `HttpConnector` that gives every connection a deadline when `max_age` is set.
#[derive(Clone)]
pub struct AgedConnector {
    inner: HttpConnector,
    max_age: Option<Duration>,
}

impl AgedConnector {
    pub fn new(inner: HttpConnector, max_age: Option<Duration>) -> Self {
        Self { inner, max_age }
    }
}

impl Service<Uri> for AgedConnector {
    type Response = AgedStream;
    type Error = ConnectError;
    type Future = Pin<Box<dyn Future<Output = Result<AgedStream, ConnectError>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, dst: Uri) -> Self::Future {
        let connecting = self.inner.call(dst);
        let max_age = self.max_age;
        Box::pin(async move {
            let io = connecting.await?;
            // up to 10% early, so connections opened together don't all expire together
            let deadline = max_age.map(|age| {
                ConnectionDeadline(
                    Instant::now() + age.mul_f64(rand::rng().random_range(0.9..=1.0)),
                )
            });
            Ok(AgedStream { io, deadline })
        })
    }
}

/// Pass-through TCP stream that adds the deadline in `connected()`.
pub struct AgedStream {
    io: TokioIo<TcpStream>,
    deadline: Option<ConnectionDeadline>,
}

impl Connection for AgedStream {
    fn connected(&self) -> Connected {
        let connected = self.io.connected();
        match self.deadline {
            Some(deadline) => connected.extra(deadline),
            None => connected,
        }
    }
}

impl Read for AgedStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: ReadBufCursor<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().io).poll_read(cx, buf)
    }
}

impl Write for AgedStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().io).poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().io).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().io).poll_shutdown(cx)
    }

    fn is_write_vectored(&self) -> bool {
        self.io.is_write_vectored()
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().io).poll_write_vectored(cx, bufs)
    }
}

/// Sends `req`, poisoning its connection when it's past its deadline (if `check_age`) or when
/// `timeout` runs out. The age check runs when the request ends, finished or dropped, so it
/// doesn't cost an extra wakeup per request. Like `tokio::time::timeout`, the outer `Err`
/// means the timeout ran out.
pub async fn request_and_retire(
    client: &HttpClient,
    mut req: Request<Full<Bytes>>,
    timeout: Option<Duration>,
    check_age: bool,
) -> Result<Result<Response<Incoming>, hyper_util::client::legacy::Error>, Elapsed> {
    let connection = AgeCheck {
        connection: capture_connection(&mut req),
        check_age,
    };
    let result = match timeout {
        Some(timeout) => tokio::time::timeout(timeout, client.request(req)).await,
        None => Ok(client.request(req).await),
    };
    if result.is_err() {
        if let Some(connected) = connection.connection.connection_metadata().as_ref() {
            connected.poison();
        }
    }
    result
}

/// Checks the connection's age when the request is done, or dropped while still running.
struct AgeCheck {
    connection: CaptureConnection,
    check_age: bool,
}

impl Drop for AgeCheck {
    fn drop(&mut self) {
        if !self.check_age {
            return;
        }
        if let Some(connected) = self.connection.connection_metadata().as_ref() {
            if is_past_deadline(connected) {
                connected.poison();
            }
        }
    }
}

fn is_past_deadline(connected: &Connected) -> bool {
    let mut extras = Extensions::new();
    connected.get_extras(&mut extras);
    extras
        .get::<ConnectionDeadline>()
        .is_some_and(|deadline| Instant::now() >= deadline.0)
}

#[cfg(test)]
mod tests {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    use std::time::Duration;

    use bytes::Bytes;
    use http_body_util::{BodyExt, Full};
    use hyper::service::service_fn;
    use hyper_util::rt::{TokioExecutor, TokioIo};
    use tokio::net::TcpListener;

    use crate::config::traffic_shaping::{
        HttpClientMode, SupergraphTrafficShapingConfig, TrafficShapingExecutorGlobalConfig,
    };
    use crate::executor::executors::http::HttpClientProvider;

    /// Local subgraph that counts accepted TCP connections. Requests with an `x-hang` header
    /// never get a response, like a pod that's stuck, and ones with `x-slow` take 300ms.
    async fn start_server(http2: bool) -> (String, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let accepted = Arc::new(AtomicUsize::new(0));
        let counter = accepted.clone();
        tokio::spawn(async move {
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                counter.fetch_add(1, Ordering::SeqCst);
                let service = service_fn(|req: hyper::Request<hyper::body::Incoming>| async move {
                    if req.headers().contains_key("x-slow") {
                        tokio::time::sleep(Duration::from_millis(300)).await;
                    }
                    if req.headers().contains_key("x-hang") {
                        std::future::pending::<()>().await;
                    }
                    Ok::<_, hyper::Error>(hyper::Response::new(Full::new(Bytes::from("ok"))))
                });
                let io = TokioIo::new(stream);
                if http2 {
                    tokio::spawn(
                        hyper::server::conn::http2::Builder::new(TokioExecutor::new())
                            .serve_connection(io, service),
                    );
                } else {
                    tokio::spawn(
                        hyper::server::conn::http1::Builder::new().serve_connection(io, service),
                    );
                }
            }
        });
        (url, accepted)
    }

    fn client(max_age: Option<Duration>, http2: bool) -> HttpClientProvider {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let traffic_shaping = SupergraphTrafficShapingConfig {
            http_client_mode: HttpClientMode::Shared,
            all: TrafficShapingExecutorGlobalConfig {
                max_connection_age: max_age,
                ..Default::default()
            },
            ..Default::default()
        };
        let pool_idle_timeout = traffic_shaping.all.pool_idle_timeout;
        HttpClientProvider::new(&traffic_shaping, None, pool_idle_timeout, http2).unwrap()
    }

    async fn get(client: &HttpClientProvider, url: &str) {
        let req = hyper::Request::get(url)
            .body(Full::new(Bytes::new()))
            .unwrap();
        let res = client.request(req, None).await.unwrap().unwrap();
        // read the body so an HTTP/1 connection goes back to the pool
        res.into_body().collect().await.unwrap();
    }

    /// Sends 6 requests 50ms apart and returns how many TCP connections the server accepted.
    async fn connections_used(max_age: Option<Duration>) -> usize {
        let (url, accepted) = start_server(false).await;
        let client = client(max_age, false);
        for _ in 0..6 {
            get(&client, &url).await;
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        accepted.load(Ordering::SeqCst)
    }

    #[tokio::test]
    async fn keeps_one_connection_without_max_age() {
        assert_eq!(connections_used(None).await, 1);
    }

    #[tokio::test]
    async fn replaces_connections_past_max_age() {
        // ~300ms of traffic with a ~100ms max age should go over at least two connections
        assert!(connections_used(Some(Duration::from_millis(100))).await >= 2);
    }

    #[tokio::test]
    async fn retires_old_http2_connection_even_when_the_request_hangs() {
        let (url, accepted) = start_server(true).await;
        let client = client(Some(Duration::from_millis(100)), true);

        get(&client, &url).await;
        tokio::time::sleep(Duration::from_millis(150)).await;

        // the connection is past its age now, and this request never gets a response
        let req = hyper::Request::get(&url)
            .header("x-hang", "1")
            .body(Full::new(Bytes::new()))
            .unwrap();
        // cancel it from outside, so only the age check can retire the connection
        let timed_out =
            tokio::time::timeout(Duration::from_millis(50), client.request(req, None)).await;
        assert!(timed_out.is_err());

        // so the next request has to open a new connection
        get(&client, &url).await;
        assert_eq!(accepted.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn retires_http2_connection_when_a_request_times_out() {
        let (url, accepted) = start_server(true).await;
        let client = client(None, true);

        get(&client, &url).await;

        let req = hyper::Request::get(&url)
            .header("x-hang", "1")
            .body(Full::new(Bytes::new()))
            .unwrap();
        let timed_out = client.request(req, Some(Duration::from_millis(50))).await;
        assert!(timed_out.is_err());

        // the stuck connection is retired, so the next request opens a new one
        get(&client, &url).await;
        assert_eq!(accepted.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn retiring_an_http2_connection_lets_its_running_requests_finish() {
        let (url, accepted) = start_server(true).await;
        let client = client(None, true);

        get(&client, &url).await;

        let slow = hyper::Request::get(&url)
            .header("x-slow", "1")
            .body(Full::new(Bytes::new()))
            .unwrap();
        let hang = hyper::Request::get(&url)
            .header("x-hang", "1")
            .body(Full::new(Bytes::new()))
            .unwrap();
        // both go over the same connection, and the stuck one times out while the slow one runs
        let (slow, hang) =
            tokio::join!(client.request(slow, Some(Duration::from_secs(5))), async {
                tokio::time::sleep(Duration::from_millis(20)).await;
                client.request(hang, Some(Duration::from_millis(50))).await
            });
        assert!(hang.is_err());
        assert_eq!(accepted.load(Ordering::SeqCst), 1);

        // the slow one still gets its whole response over the retired connection
        let body = slow.unwrap().unwrap().into_body().collect().await.unwrap();
        assert_eq!(body.to_bytes(), "ok");

        // and new requests open a fresh connection
        get(&client, &url).await;
        assert_eq!(accepted.load(Ordering::SeqCst), 2);
    }
}
