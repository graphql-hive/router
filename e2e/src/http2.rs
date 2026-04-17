#[cfg(test)]
mod http2_tests {
    use std::io::Write;

    use axum_server::tls_rustls::RustlsConfig;
    use hive_router::init_rustls_crypto_provider;
    use rcgen::generate_simple_self_signed;
    use sonic_rs::json;
    use tempfile::NamedTempFile;

    use crate::testkit::{ClientResponseExt, Started, TestRouter, TestSubgraphs};

    struct GeneratedKeyPair {
        _cert_file: NamedTempFile,
        cert_file_path: String,
        cert_pem: String,
        _key_file: NamedTempFile,
        key_file_path: String,
    }

    async fn generate_keypair() -> GeneratedKeyPair {
        let cert_key = generate_simple_self_signed(vec![
            "127.0.0.1".to_string(),
            "localhost".to_string(),
            "0.0.0.0".to_string(),
        ])
        .expect("Failed to generate self-signed certificate");

        let mut cert_file =
            NamedTempFile::new().expect("Failed to create temporary file for certificate");
        let cert = cert_key.cert;
        let cert_pem = cert.pem();
        cert_file
            .write(cert_pem.as_bytes())
            .expect("Failed to write certificate to temporary file");

        let mut key_file =
            NamedTempFile::new().expect("Failed to create temporary file for private key");
        let key = cert_key.signing_key;
        let key_str = key.serialize_pem();
        key_file
            .write(key_str.as_bytes())
            .expect("Failed to write private key to temporary file");

        GeneratedKeyPair {
            cert_file_path: cert_file
                .path()
                .to_str()
                .expect("Failed to convert cert file path to string")
                .to_string(),
            _cert_file: cert_file,
            cert_pem,
            key_file_path: key_file
                .path()
                .to_str()
                .expect("Failed to convert key file path to string")
                .to_string(),
            _key_file: key_file,
        }
    }

    async fn generate_tls_subgraph() -> (TestSubgraphs<Started>, GeneratedKeyPair) {
        let generated_key_pair = generate_keypair().await;
        let rustls_config = RustlsConfig::from_pem_file(
            &generated_key_pair.cert_file_path,
            &generated_key_pair.key_file_path,
        )
        .await
        .expect("Failed to create RustlsConfig from PEM files");
        let subgraphs = TestSubgraphs::builder()
            .with_rustls_config(rustls_config)
            .build()
            .start()
            .await;
        (subgraphs, generated_key_pair)
    }

    /// Verify that a client can communicate with the router using HTTP/2 over TLS.
    /// The reqwest client with rustls and http2 features will negotiate h2 via ALPN.
    #[ntex::test]
    async fn client_to_router_http2_over_tls() {
        init_rustls_crypto_provider();
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let generated_key_pair = generate_keypair().await;

        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(format!(
                r#"
            supergraph:
                source: file
                path: supergraph.graphql
            traffic_shaping:
                router:
                    tls:
                        key_file: "{}"
                        cert_file: "{}"
                "#,
                generated_key_pair.key_file_path, generated_key_pair.cert_file_path
            ))
            .build()
            .start_without_healthcheck()
            .await;

        let graphql_endpoint = router.serv().url(router.graphql_path());

        // Build a reqwest client that trusts the self-signed cert.
        // reqwest with http2+rustls-tls will negotiate h2 via ALPN automatically.
        let client = reqwest::Client::builder()
            .add_root_certificate(
                reqwest::Certificate::from_pem(generated_key_pair.cert_pem.as_bytes())
                    .expect("Failed to create certificate from PEM"),
            )
            .use_rustls_tls()
            .build()
            .expect("Failed to build reqwest client");

        let resp = client
            .post(&graphql_endpoint)
            .json(&json!({
                "query": "{ me { name } }"
            }))
            .send()
            .await
            .expect("Failed to send HTTP/2 request to router");

        assert_eq!(
            resp.version(),
            reqwest::Version::HTTP_2,
            "Expected HTTP/2 but got {:?}",
            resp.version()
        );
        assert!(resp.status().is_success());

        let body = resp.text().await.expect("Failed to read response body");
        assert!(
            body.contains("Uri Goldshtein"),
            "Response should contain expected data, got: {}",
            body
        );
    }

    /// Verify that the router communicates with TLS-enabled subgraphs using HTTP/2.
    /// The router's hyper-rustls connector uses enable_all_versions() which negotiates h2 via ALPN.
    #[ntex::test]
    async fn router_to_subgraph_http2_over_tls() {
        init_rustls_crypto_provider();
        let (subgraphs, subgraph_keypair) = generate_tls_subgraph().await;

        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(format!(
                r#"
            supergraph:
                source: file
                path: supergraph.graphql
            traffic_shaping:
                subgraphs:
                    accounts:
                        tls:
                            cert_file: "{}"
                "#,
                subgraph_keypair.cert_file_path
            ))
            .build()
            .start()
            .await;

        let resp = router
            .send_graphql_request("{ me { name } }", None, None)
            .await;

        assert!(resp.status().is_success(), "Expected 200 OK");

        // Check that the subgraph received the request over HTTP/2
        let subgraph_requests = subgraphs
            .get_requests_log("accounts")
            .expect("Expected requests sent to accounts subgraph");
        assert_eq!(
            subgraph_requests.len(),
            1,
            "Expected exactly 1 request to accounts subgraph"
        );
        assert_eq!(
            subgraph_requests[0].http_version,
            http::Version::HTTP_2,
            "Expected router→subgraph request to use HTTP/2, got {:?}",
            subgraph_requests[0].http_version
        );
    }

    /// Verify full HTTP/2 path: Client --h2--> Router --h2--> Subgraph (both directions over TLS).
    #[ntex::test]
    async fn full_http2_path_client_router_subgraph() {
        init_rustls_crypto_provider();
        let (subgraphs, subgraph_keypair) = generate_tls_subgraph().await;
        let router_keypair = generate_keypair().await;

        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(format!(
                r#"
            supergraph:
                source: file
                path: supergraph.graphql
            traffic_shaping:
                router:
                    tls:
                        key_file: "{}"
                        cert_file: "{}"
                subgraphs:
                    accounts:
                        tls:
                            cert_file: "{}"
                "#,
                router_keypair.key_file_path,
                router_keypair.cert_file_path,
                subgraph_keypair.cert_file_path
            ))
            .build()
            .start_without_healthcheck()
            .await;

        let graphql_endpoint = router.serv().url(router.graphql_path());

        let client = reqwest::Client::builder()
            .add_root_certificate(
                reqwest::Certificate::from_pem(router_keypair.cert_pem.as_bytes())
                    .expect("Failed to create certificate from PEM"),
            )
            .use_rustls_tls()
            .build()
            .expect("Failed to build reqwest client");

        let resp = client
            .post(&graphql_endpoint)
            .json(&json!({
                "query": "{ me { name } }"
            }))
            .send()
            .await
            .expect("Failed to send request");

        // Verify Client → Router is HTTP/2
        assert_eq!(
            resp.version(),
            reqwest::Version::HTTP_2,
            "Client→Router should use HTTP/2, got {:?}",
            resp.version()
        );
        assert!(resp.status().is_success());

        let body = resp.text().await.expect("Failed to read response body");
        assert!(
            body.contains("Uri Goldshtein"),
            "Response should contain expected data, got: {}",
            body
        );

        // Verify Router → Subgraph is HTTP/2
        let subgraph_requests = subgraphs
            .get_requests_log("accounts")
            .expect("Expected requests sent to accounts subgraph");
        assert_eq!(subgraph_requests.len(), 1);
        assert_eq!(
            subgraph_requests[0].http_version,
            http::Version::HTTP_2,
            "Router→Subgraph should use HTTP/2, got {:?}",
            subgraph_requests[0].http_version
        );
    }

    /// Verify that plain HTTP (no TLS) connections use HTTP/1.1 (h2c is not enabled by default).
    #[ntex::test]
    async fn plain_http_uses_http1() {
        let subgraphs = TestSubgraphs::builder().build().start().await;

        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
            supergraph:
                source: file
                path: supergraph.graphql
                "#
                .to_string(),
            )
            .build()
            .start()
            .await;

        let resp = router
            .send_graphql_request("{ me { name } }", None, None)
            .await;
        assert!(resp.status().is_success());

        // Without TLS, subgraph should be reached via HTTP/1.1
        let subgraph_requests = subgraphs
            .get_requests_log("accounts")
            .expect("Expected requests sent to accounts subgraph");
        assert_eq!(subgraph_requests.len(), 1);
        assert_eq!(
            subgraph_requests[0].http_version,
            http::Version::HTTP_11,
            "Plain HTTP should use HTTP/1.1, got {:?}",
            subgraph_requests[0].http_version
        );
    }

    /// Verify that h2c (HTTP/2 cleartext) works when http2_only is enabled globally for all subgraphs.
    #[ntex::test]
    async fn h2c_router_to_subgraph_global_config() {
        let subgraphs = TestSubgraphs::builder()
            .with_http2_only()
            .build()
            .start()
            .await;

        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
            supergraph:
                source: file
                path: supergraph.graphql
            traffic_shaping:
                all:
                    http2_only: true
                "#
                .to_string(),
            )
            .build()
            .start()
            .await;

        let resp = router
            .send_graphql_request("{ me { name } }", None, None)
            .await;
        assert!(resp.status().is_success());

        let body = resp.string_body().await;
        assert!(
            body.contains("Uri Goldshtein"),
            "Response should contain expected data, got: {}",
            body
        );

        // Verify the subgraph received the request over HTTP/2 (h2c)
        let subgraph_requests = subgraphs
            .get_requests_log("accounts")
            .expect("Expected requests sent to accounts subgraph");
        assert_eq!(subgraph_requests.len(), 1);
        assert_eq!(
            subgraph_requests[0].http_version,
            http::Version::HTTP_2,
            "h2c: Router→Subgraph should use HTTP/2, got {:?}",
            subgraph_requests[0].http_version
        );
    }

    /// Verify that h2c works when http2_only is enabled for a specific subgraph.
    #[ntex::test]
    async fn h2c_router_to_subgraph_per_subgraph_config() {
        let subgraphs = TestSubgraphs::builder()
            .with_http2_only()
            .build()
            .start()
            .await;

        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
            supergraph:
                source: file
                path: supergraph.graphql
            traffic_shaping:
                subgraphs:
                    accounts:
                        http2_only: true
                "#
                .to_string(),
            )
            .build()
            .start()
            .await;

        let resp = router
            .send_graphql_request("{ me { name } }", None, None)
            .await;
        assert!(resp.status().is_success());

        let body = resp.string_body().await;
        assert!(
            body.contains("Uri Goldshtein"),
            "Response should contain expected data, got: {}",
            body
        );

        // Verify the subgraph received the request over HTTP/2 (h2c)
        let subgraph_requests = subgraphs
            .get_requests_log("accounts")
            .expect("Expected requests sent to accounts subgraph");
        assert_eq!(subgraph_requests.len(), 1);
        assert_eq!(
            subgraph_requests[0].http_version,
            http::Version::HTTP_2,
            "h2c: Router→Subgraph should use HTTP/2, got {:?}",
            subgraph_requests[0].http_version
        );
    }

    /// Verify that without http2_only flag, plain HTTP to an h2c subgraph fails
    /// (the router defaults to HTTP/1.1 and the h2c-only subgraph rejects it).
    #[ntex::test]
    async fn h2c_subgraph_rejects_http1_without_flag() {
        let subgraphs = TestSubgraphs::builder()
            .with_http2_only()
            .build()
            .start()
            .await;

        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
            supergraph:
                source: file
                path: supergraph.graphql
                "#
                .to_string(),
            )
            .build()
            .start()
            .await;

        let resp = router
            .send_graphql_request("{ me { name } }", None, None)
            .await;

        // The request should fail because the router sent HTTP/1.1
        // but the subgraph only accepts HTTP/2
        let body = resp.string_body().await;
        assert!(
            body.contains("error") || body.contains("FETCH_ERROR"),
            "Expected an error when sending HTTP/1.1 to h2c-only subgraph, got: {}",
            body
        );
    }
}
