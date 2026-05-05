use std::{convert::Infallible, env, path::Path, sync::Arc};

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{
    body::Incoming,
    header::{CONTENT_LENGTH, CONTENT_TYPE},
    server::conn::http2,
    service::service_fn,
    Method, Request, Response, StatusCode,
};
use hyper_util::rt::{TokioExecutor, TokioIo};
use tokio::{fs, net::UnixListener};

const DEFAULT_SOCKET_PATH: &str = "/tmp/hive-bench-coprocessor.sock";
const DEFAULT_COPROCESSOR_PATH: &str = "/coprocessor";
const CONTINUE_RESPONSE: &[u8] =
    br#"{"version":1,"control":"continue","context":{"custom::a":"bench-coprocessor"}}"#;

#[tokio::main]
async fn main() {
    let socket_path = env::var("COPROCESSOR_SOCKET_PATH")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_SOCKET_PATH.to_string());
    let coprocessor_path = env::var("COPROCESSOR_PATH")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_COPROCESSOR_PATH.to_string());
    let coprocessor_path: Arc<str> = coprocessor_path.into();

    if Path::new(&socket_path).exists() {
        let _ = fs::remove_file(&socket_path).await;
    }

    let listener = UnixListener::bind(&socket_path).expect("failed to bind unix socket");
    println!(
        "bench-coprocessor listening on unix://{} (h2c, path {})",
        socket_path, coprocessor_path
    );

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                break;
            }
            accepted = listener.accept() => {
                let (stream, _) = match accepted {
                    Ok(value) => value,
                    Err(err) => {
                        eprintln!("failed to accept unix connection: {err}");
                        continue;
                    }
                };

                let coprocessor_path = Arc::clone(&coprocessor_path);
                tokio::spawn(async move {
                    let service = service_fn(move |req| handle_request(req, Arc::clone(&coprocessor_path)));
                    let io = TokioIo::new(stream);

                    if let Err(err) = http2::Builder::new(TokioExecutor::new())
                        .serve_connection(io, service)
                        .await
                    {
                        eprintln!("h2c connection error: {err}");
                    }
                });
            }
        }
    }

    let _ = fs::remove_file(&socket_path).await;
}

async fn handle_request(
    request: Request<Incoming>,
    coprocessor_path: Arc<str>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    if request.method() != Method::POST || request.uri().path() != coprocessor_path.as_ref() {
        return Ok(response(StatusCode::NOT_FOUND, &[]));
    }

    let _ = match request.into_body().collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(err) => {
            eprintln!("failed to read request body: {err}");
            return Ok(response(StatusCode::BAD_REQUEST, &[]));
        }
    };
    Ok(response(StatusCode::OK, CONTINUE_RESPONSE))
}

fn response(status: StatusCode, body: &'static [u8]) -> Response<Full<Bytes>> {
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, "application/json")
        .header(CONTENT_LENGTH, body.len())
        .body(Full::new(Bytes::from_static(body)))
        .expect("failed to build response")
}
