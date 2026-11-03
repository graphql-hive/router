use std::{
    net::SocketAddr,
    sync::{Arc, Mutex},
};

use axum::{extract::State, routing::post, Json, Router};
use axum_server::{tls_rustls::RustlsConfig, Handle};
use hive_router::init_rustls_crypto_provider;
use http::Version;
use rustls::{
    pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer},
    server::WebPkiClientVerifier,
    RootCertStore, ServerConfig,
};
use serde_json::{json, Value};

use crate::testkit::{generate_keypair, GeneratedKeyPair, Started, TestRouter, TestSubgraphs};

struct TlsCoprocessor {
    addr: SocketAddr,
    handle: Handle<SocketAddr>,
    versions: Arc<Mutex<Vec<Version>>>,
}

impl TlsCoprocessor {
    async fn start(rustls_config: RustlsConfig) -> Self {
        let versions = Arc::new(Mutex::new(Vec::new()));
        let app = Router::new()
            .route("/coprocessor", post(coprocessor_handler))
            .with_state(versions.clone());

        let handle = Handle::new();
        let server_handle = handle.clone();
        tokio::spawn(async move {
            axum_server::bind_rustls("127.0.0.1:0".parse().unwrap(), rustls_config)
                .handle(server_handle)
                .serve(app.into_make_service())
                .await
                .expect("failed to start tls coprocessor");
        });

        let addr = handle
            .listening()
            .await
            .expect("failed to get tls coprocessor address");

        Self {
            addr,
            handle,
            versions,
        }
    }

    fn url(&self) -> String {
        format!("https://{}/coprocessor", self.addr)
    }

    fn versions(&self) -> Vec<Version> {
        self.versions.lock().unwrap().clone()
    }
}

impl Drop for TlsCoprocessor {
    fn drop(&mut self) {
        self.handle.shutdown();
    }
}

async fn coprocessor_handler(
    State(versions): State<Arc<Mutex<Vec<Version>>>>,
    version: Version,
    Json(payload): Json<Value>,
) -> Json<Value> {
    versions.lock().unwrap().push(version);

    if payload.get("stage").and_then(Value::as_str) == Some("router.response") {
        return Json(json!({
            "version": 1,
            "control": "continue",
            "headers": {
                "content-type": "application/json",
                "x-coprocessor-response": "injected-over-tls"
            }
        }));
    }

    Json(json!({ "version": 1, "control": "continue" }))
}

fn router_config(url: &str, protocol: &str, tls: &str) -> String {
    format!(
        r#"
            supergraph:
              source: file
              path: supergraph.graphql
            coprocessor:
              url: {url}
              protocol: {protocol}
{tls}
              stages:
                router:
                  response:
                    include:
                      status_code: true
            "#
    )
}

fn trust_cert_tls(server_key_pair: &GeneratedKeyPair) -> String {
    format!(
        r#"
              tls:
                cert_file: "{}"
"#,
        server_key_pair.cert_file_path
    )
}

/// Sends a request through the router and returns whether the coprocessor's response hook ran.
async fn coprocessor_reached(router: &TestRouter<Started>) -> bool {
    let response = router
        .send_graphql_request("{ topProducts { name } }", None, None)
        .await;

    response.status().is_success()
        && response
            .headers()
            .get("x-coprocessor-response")
            .and_then(|v| v.to_str().ok())
            == Some("injected-over-tls")
}

async fn assert_works_over_https(protocol: &str, expected_version: Version) {
    init_rustls_crypto_provider();
    let subgraphs = TestSubgraphs::builder().build().start().await;
    let server_key_pair = generate_keypair().await;
    let rustls_config = RustlsConfig::from_pem_file(
        &server_key_pair.cert_file_path,
        &server_key_pair.key_file_path,
    )
    .await
    .expect("failed to create rustls config");
    let coprocessor = TlsCoprocessor::start(rustls_config).await;

    let router = TestRouter::builder()
        .with_subgraphs(&subgraphs)
        .inline_config(router_config(
            &coprocessor.url(),
            protocol,
            &trust_cert_tls(&server_key_pair),
        ))
        .build()
        .start()
        .await;

    assert!(coprocessor_reached(&router).await);
    assert_eq!(coprocessor.versions(), vec![expected_version]);
}

#[ntex::test]
async fn works_over_https_http1() {
    assert_works_over_https("http1", Version::HTTP_11).await;
}

#[ntex::test]
async fn works_over_https_http2() {
    assert_works_over_https("http2", Version::HTTP_2).await;
}

#[ntex::test]
async fn rejects_untrusted_certificate_by_default() {
    init_rustls_crypto_provider();
    let subgraphs = TestSubgraphs::builder().build().start().await;
    let server_key_pair = generate_keypair().await;
    let rustls_config = RustlsConfig::from_pem_file(
        &server_key_pair.cert_file_path,
        &server_key_pair.key_file_path,
    )
    .await
    .expect("failed to create rustls config");
    let coprocessor = TlsCoprocessor::start(rustls_config).await;

    // No `tls` block: the self-signed certificate is not in the system's root certificates.
    let router = TestRouter::builder()
        .with_subgraphs(&subgraphs)
        .inline_config(router_config(&coprocessor.url(), "http1", ""))
        .build()
        .start()
        .await;

    let response = router
        .send_graphql_request("{ topProducts { name } }", None, None)
        .await;

    assert_eq!(response.status().as_u16(), 500);
    assert!(coprocessor.versions().is_empty());
}

#[ntex::test]
async fn insecure_skip_ca_verification_accepts_untrusted_certificate() {
    init_rustls_crypto_provider();
    let subgraphs = TestSubgraphs::builder().build().start().await;
    let server_key_pair = generate_keypair().await;
    let rustls_config = RustlsConfig::from_pem_file(
        &server_key_pair.cert_file_path,
        &server_key_pair.key_file_path,
    )
    .await
    .expect("failed to create rustls config");
    let coprocessor = TlsCoprocessor::start(rustls_config).await;

    let router = TestRouter::builder()
        .with_subgraphs(&subgraphs)
        .inline_config(router_config(
            &coprocessor.url(),
            "http1",
            r#"
              tls:
                insecure_skip_ca_verification: true
"#,
        ))
        .build()
        .start()
        .await;

    assert!(coprocessor_reached(&router).await);
}

#[ntex::test]
async fn mtls() {
    init_rustls_crypto_provider();
    let subgraphs = TestSubgraphs::builder().build().start().await;
    let server_key_pair = generate_keypair().await;
    let client_key_pair = generate_keypair().await;

    let mut client_roots = RootCertStore::empty();
    client_roots
        .add(
            CertificateDer::from_pem_file(&client_key_pair.cert_file_path)
                .expect("failed to read client certificate"),
        )
        .expect("failed to add client certificate to root store");

    let mut server_config = ServerConfig::builder()
        .with_client_cert_verifier(
            WebPkiClientVerifier::builder(client_roots.into())
                .build()
                .expect("failed to build client verifier"),
        )
        .with_single_cert(
            vec![
                CertificateDer::from_pem_file(&server_key_pair.cert_file_path)
                    .expect("failed to read server certificate"),
            ],
            PrivateKeyDer::from_pem_file(&server_key_pair.key_file_path)
                .expect("failed to read server key"),
        )
        .expect("failed to build server config");
    server_config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    let coprocessor =
        TlsCoprocessor::start(RustlsConfig::from_config(Arc::new(server_config))).await;

    let router_with_client_cert = TestRouter::builder()
        .with_subgraphs(&subgraphs)
        .inline_config(router_config(
            &coprocessor.url(),
            "http1",
            &format!(
                r#"
              tls:
                cert_file: "{}"
                client_auth:
                  cert_file: "{}"
                  key_file: "{}"
"#,
                server_key_pair.cert_file_path,
                client_key_pair.cert_file_path,
                client_key_pair.key_file_path
            ),
        ))
        .build()
        .start()
        .await;

    assert!(coprocessor_reached(&router_with_client_cert).await);

    let router_without_client_cert = TestRouter::builder()
        .with_subgraphs(&subgraphs)
        .inline_config(router_config(
            &coprocessor.url(),
            "http1",
            &trust_cert_tls(&server_key_pair),
        ))
        .build()
        .start()
        .await;

    assert!(!coprocessor_reached(&router_without_client_cert).await);
    assert_eq!(coprocessor.versions(), vec![Version::HTTP_11]);
}
