use std::{net::SocketAddr, path::PathBuf};

use axum::http::{Response, StatusCode};
use axum::{error_handling::HandleError, Router};
use s3s::Body;
use s3s::{auth::SimpleAuth, service::S3ServiceBuilder, HttpError};
use s3s_fs::FileSystem;
use tempfile::TempDir;
use tokio::{net::TcpListener, sync::oneshot, task::JoinHandle};

pub const MOCK_ACCESS_KEY: &str = "AKIAIOSFODNN7EXAMPLE";
pub const MOCK_SECRET_KEY: &str = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";

pub struct S3Mock {
    /// The base URL of the mock service, e.g. `http://127.0.0.1:54321`.
    url: String,
    /// Root of the bucket on disk.
    root: PathBuf,
    bucket: String,
    _temp_dir: TempDir,
    shutdown_tx: Option<oneshot::Sender<()>>,
    server_handle: Option<JoinHandle<()>>,
}

async fn handle_s3_error(err: HttpError) -> Response<Body> {
    tracing::error!(?err, "S3 service error");
    Response::builder()
        .status(StatusCode::INTERNAL_SERVER_ERROR)
        .body(Body::from("Internal Server Error".to_string()))
        .unwrap()
}

impl S3Mock {
    pub async fn start(bucket: &str) -> Self {
        let temp_dir = tempfile::tempdir().expect("no temp dir available");
        let root = temp_dir.path().to_path_buf();

        let bucket_path = root.join(bucket);
        tokio::fs::create_dir_all(&bucket_path)
            .await
            .expect("failed to create bucket directory");

        let fs = FileSystem::new(&root).expect("failed to create s3s filesystem");
        let auth = SimpleAuth::from_single(MOCK_ACCESS_KEY, MOCK_SECRET_KEY);
        let s3_service = {
            let mut b = S3ServiceBuilder::new(fs);
            b.set_auth(auth);
            b.build()
        };

        let s3_service = HandleError::new(s3_service, handle_s3_error);
        let router = Router::new().fallback_service(s3_service);

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("failed to create listener");
        let addr: SocketAddr = listener.local_addr().expect("failed to get local addr");
        let url = format!("http://{addr}");
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        let server_handle = tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                })
                .await
                .expect("s3 mock server error");
        });

        Self {
            url,
            root: root.clone(),
            bucket: bucket.to_owned(),
            _temp_dir: temp_dir,
            shutdown_tx: Some(shutdown_tx),
            server_handle: Some(server_handle),
        }
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn bucket(&self) -> &str {
        &self.bucket
    }

    pub fn access_key(&self) -> &str {
        MOCK_ACCESS_KEY
    }

    pub fn secret_key(&self) -> &str {
        MOCK_SECRET_KEY
    }

    pub async fn set(&self, key: &str, contents: &[u8]) {
        let dest = self.object_path(key);
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .expect("failed to create parent dirs for mock object");
        }
        tokio::fs::write(&dest, contents)
            .await
            .expect("failed to write mock object");
    }

    pub async fn remove(&self, key: &str) {
        let path = self.object_path(key);
        let _ = tokio::fs::remove_file(path).await;
    }

    fn object_path(&self, key: &str) -> PathBuf {
        let key = key.trim_start_matches('/');
        self.root.join(&self.bucket).join(key)
    }
}

impl Drop for S3Mock {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }

        if let Some(handle) = self.server_handle.take() {
            handle.abort();
        }
    }
}
