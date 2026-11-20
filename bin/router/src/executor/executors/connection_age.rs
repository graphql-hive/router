//! Retires subgraph connections once they hit `max_connection_age`.
//!
//! A busy connection never sits idle, so `pool_idle_timeout` never closes it, and it stays on
//! whatever pod it first connected to. Giving every connection a deadline makes the pool open
//! fresh ones from time to time, which re-resolves DNS and lets the Service pick a pod again.
//!
//! The connector stamps each new connection with a deadline. When a request picks up a connection
//! past its deadline, we poison it, so the pool stops handing it out. That request and any others
//! already on it finish normally, then it closes.

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
    capture_connection, Connected, Connection, HttpConnector,
};
use hyper_util::rt::TokioIo;
use rand::prelude::*;
use tokio::net::TcpStream;
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
            // Shave up to 10% off so connections opened together (e.g. at startup)
            // don't all expire and reconnect in the same instant.
            let deadline = max_age.map(|age| {
                ConnectionDeadline(
                    Instant::now() + age.mul_f64(rand::rng().random_range(0.9..=1.0)),
                )
            });
            Ok(AgedStream { io, deadline })
        })
    }
}

/// Plain pass-through around the TCP stream. It only exists to add the deadline in `connected()`.
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

/// Sends `req` and retires the connection it goes over if that connection is past its deadline.
///
/// The check runs as soon as the request gets a connection, not when the response arrives, so
/// a request that hangs or times out still stops others from reusing an old connection. The
/// request itself still goes over that connection.
///
/// A connection only gets checked when it carries a request. One that goes quiet past its
/// deadline is closed by `pool_idle_timeout` instead.
pub async fn request_and_retire_old(
    client: &HttpClient,
    mut req: Request<Full<Bytes>>,
) -> Result<Response<Incoming>, hyper_util::client::legacy::Error> {
    let mut connection = capture_connection(&mut req);
    let request = client.request(req);
    tokio::pin!(request);
    // Don't just wait for the connection: if connecting fails, it never shows up.
    let finished = tokio::select! {
        res = &mut request => Some(res),
        _ = connection.wait_for_connection_metadata() => None,
    };
    if let Some(connected) = connection.connection_metadata().as_ref() {
        let mut extras = Extensions::new();
        connected.get_extras(&mut extras);
        if extras
            .get::<ConnectionDeadline>()
            .is_some_and(|deadline| Instant::now() >= deadline.0)
        {
            connected.poison();
        }
    }
    match finished {
        Some(res) => res,
        None => request.await,
    }
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
    use hyper_util::client::legacy::Client;
    use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
    use tokio::net::TcpListener;

    use crate::config::traffic_shaping::HttpClientMode;
    use crate::executor::executors::http::HttpClientProvider;
    use crate::executor::executors::tls::build_https_connector;

    /// Local subgraph that counts accepted TCP connections. Requests with an `x-hang` header
    /// never get a response, like a pod that's stuck.
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
        let mut builder = Client::builder(TokioExecutor::new());
        builder.pool_timer(TokioTimer::new()).http2_only(http2);
        HttpClientProvider::new(
            HttpClientMode::Shared,
            builder,
            build_https_connector(None, max_age, None).unwrap(),
            max_age.is_some(),
        )
    }

    async fn get(client: &HttpClientProvider, url: &str) {
        let req = hyper::Request::get(url)
            .body(Full::new(Bytes::new()))
            .unwrap();
        let res = client.request(req).await.unwrap();
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
    async fn retires_old_http2_connection_even_when_the_request_times_out() {
        let (url, accepted) = start_server(true).await;
        let client = client(Some(Duration::from_millis(100)), true);

        get(&client, &url).await;
        tokio::time::sleep(Duration::from_millis(150)).await;

        // the connection is past its age now, and this request never gets a response
        let req = hyper::Request::get(&url)
            .header("x-hang", "1")
            .body(Full::new(Bytes::new()))
            .unwrap();
        let timed_out = tokio::time::timeout(Duration::from_millis(50), client.request(req)).await;
        assert!(timed_out.is_err());

        // so the next request has to open a new connection
        get(&client, &url).await;
        assert_eq!(accepted.load(Ordering::SeqCst), 2);
    }
}
