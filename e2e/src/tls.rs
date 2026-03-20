#[cfg(test)]
mod tls_tests {
    use std::{io::Write, sync::Arc};

    use axum_server::tls_rustls::RustlsConfig;
    use hive_router::init_rustls_crypto_provider;
    use rcgen::generate_simple_self_signed;
    use rustls::{
        pki_types::{pem::PemObject, PrivateKeyDer},
        server::WebPkiClientVerifier,
        RootCertStore, ServerConfig,
    };
    use sonic_rs::json;
    use tempfile::NamedTempFile;
    use tonic::transport::CertificateDer;

    use crate::testkit::{ClientResponseExt, Started, TestRouter, TestSubgraphs};

    struct GeneratedKeyPair {
        cert_file: NamedTempFile,
        cert_file_path: String,
        cert_pem: String,
        key_file: NamedTempFile,
        key_file_path: String,
        key_pem: String,
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
                .expect("Failed to convert certificate file path to string")
                .to_string(),
            cert_file,
            cert_pem,
            key_file_path: key_file
                .path()
                .to_str()
                .expect("Failed to convert private key file path to string")
                .to_string(),
            key_file,
            key_pem: key_str,
        }
    }

    // Setup TLS on router
    // And send a request from a client, that has the router's certificate configured as a trusted root, to the router
    // Verify that the request succeeds, indicating that TLS is working correctly on the router
    #[ntex::test]
    async fn router_tls() {
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
        let client = reqwest::Client::builder()
            .add_root_certificate(
                reqwest::Certificate::from_pem(generated_key_pair.cert_pem.as_bytes())
                    .expect("Failed to create certificate from PEM"),
            )
            .use_rustls_tls()
            .build()
            .expect("Failed to build reqwest client with custom TLS configuration");
        let resp = client
            .post(graphql_endpoint)
            .json(&json!({
                "query": "{ me { name } }"
            }))
            .send()
            .await
            .expect("Failed to send request to router with TLS");
        insta::assert_snapshot!(
            resp.text().await.expect("Failed to parse text response from router with TLS")
            , @r#"{"data":{"me":{"name":"Uri Goldshtein"}}}"#);
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

    /// Setup TLS on a subgraph
    /// Configure the router to trust the subgraph's certificate authority
    /// Send a request to the router that requires communication with the TLS-enabled subgraph and verify that the request succeeds,
    /// indicating that TLS is working correctly between the router and the subgraph
    #[ntex::test]
    async fn overriding_cert_auth_for_subgraphs() {
        init_rustls_crypto_provider();
        let (subgraphs, generated_key_pair) = generate_tls_subgraph().await;
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
                generated_key_pair.cert_file_path
            ))
            .build()
            .start()
            .await;
        let resp = router
            .send_graphql_request("{ me { name } }", None, None)
            .await;
        assert!(resp.status().is_success(), "Expected 200 OK");
        insta::assert_snapshot!(
            resp.json_body_string_pretty().await
            , @r###"
        {
          "data": {
            "me": {
              "name": "Uri Goldshtein"
            }
          }
        }
        "###);
    }

    // Setup two subgraph servers with TLS, each with its own certificate authority
    // Configure the router to trust both certificate authorities
    // Send a request to the router that requires communication with both TLS-enabled subgraphs and verify
    // that the request succeeds, indicating that TLS is working correctly between the router and both subgraphs
    #[ntex::test]
    async fn overriding_multiple_cert_auth_for_subgraphs() {
        init_rustls_crypto_provider();
        let (subgraph1, generated_key_pair1) = generate_tls_subgraph().await;
        let (subgraph2, generated_key_pair2) = generate_tls_subgraph().await;
        let mut combined_ca_file =
            NamedTempFile::new().expect("Failed to create temporary file for certificate");
        let combined_ca_pem = format!(
            "{}\n{}",
            generated_key_pair1.cert_pem, generated_key_pair2.cert_pem
        );
        combined_ca_file
            .write(combined_ca_pem.as_bytes())
            .expect("Failed to write combined certificate to temporary file");
        let router = TestRouter::builder()
            .inline_config(format!(
                r#"
            supergraph:
                source: file
                path: supergraph.graphql
            traffic_shaping:
                all:
                    tls:
                        cert_file: "{}"
            override_subgraph_urls:
                accounts:
                    url: "{}/accounts"
                reviews:
                    url: "{}/reviews"
            "#,
                combined_ca_file
                    .path()
                    .to_str()
                    .expect("Expected to have a path for the combined ca file"),
                subgraph1.url(),
                subgraph2.url()
            ))
            .build()
            .start()
            .await;
        let resp = router
            .send_graphql_request("{ me { name reviews { body } } }", None, None)
            .await;
        assert!(resp.status().is_success(), "Expected 200 OK");
        insta::assert_snapshot!(
            resp.json_body_string_pretty().await
            , @r#"
        {
          "data": {
            "me": {
              "name": "Uri Goldshtein",
              "reviews": [
                {
                  "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum."
                },
                {
                  "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi"
                }
              ]
            }
          }
        }
        "#);
    }
    // Setup mTLS on a subgraph
    // Configure the router to communicate with the subgraph using mTLS
    // Send a request to the router that requires communication with the mTLS-enabled subgraph and
    // verify that the request succeeds, indicating that mTLS is working correctly between the router and the subgraph
    #[ntex::test]
    async fn mtls_subgraph() {
        init_rustls_crypto_provider();
        let generated_keypair = generate_keypair().await;
        let client_auth_generated_key_pair = generate_keypair().await;

        let mut client_auth_roots = RootCertStore::empty();
        let client_auth_cert: CertificateDer<'static> =
            CertificateDer::from_pem_file(client_auth_generated_key_pair.cert_file_path)
                .expect("Failed to read certificate from PEM file");
        client_auth_roots
            .add(client_auth_cert.clone())
            .expect("Failed to add certificate to root store");

        let cert = CertificateDer::from_pem_file(generated_keypair.cert_file_path)
            .expect("Failed to read certificate from PEM file");
        let key: PrivateKeyDer<'static> =
            PrivateKeyDer::from_pem_file(generated_keypair.key_file_path)
                .expect("Failed to read private key from PEM file");
        let rustls_config = RustlsConfig::from_config(Arc::new(
            ServerConfig::builder()
                .with_client_cert_verifier(
                    WebPkiClientVerifier::builder(client_auth_roots.into())
                        .build()
                        .expect("Failed to build WebPkiClientVerifier for mTLS test"),
                )
                .with_single_cert(vec![cert], key)
                .unwrap(),
        ));

        let subgraphs = TestSubgraphs::builder()
            .with_rustls_config(rustls_config)
            .build()
            .start()
            .await;
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
                            client_auth:
                                cert_file: "{}"
                                key_file: "{}"
                "#,
                generated_keypair
                    .cert_file
                    .path()
                    .to_str()
                    .expect("Failed to convert key file path to string"),
                client_auth_generated_key_pair
                    .cert_file
                    .path()
                    .to_str()
                    .expect("Failed to convert key file path to string"),
                client_auth_generated_key_pair
                    .key_file
                    .path()
                    .to_str()
                    .expect("Failed to convert key file path to string")
            ))
            .build()
            .start()
            .await;
        let resp = router
            .send_graphql_request("{ me { name } }", None, None)
            .await;
        assert!(resp.status().is_success(), "Expected 200 OK");
        insta::assert_snapshot!(
            resp.json_body_string_pretty().await
            , @r###"
        {
          "data": {
            "me": {
              "name": "Uri Goldshtein"
            }
          }
        }
        "###);
    }

    // Setup mTLS on the router
    // And setup an HTTP client with mTLS configured to communicate with the router
    // Send a request to the router and verify that it succeeds, indicating that mTLS is
    // working correctly on the router
    #[ntex::test]
    async fn mtls_router() {
        init_rustls_crypto_provider();
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let generated_key_pair = generate_keypair().await;
        let client_auth_generated_key_pair = generate_keypair().await;
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
                        client_auth:
                            cert_file: "{}"
                "#,
                generated_key_pair.key_file_path,
                generated_key_pair.cert_file_path,
                client_auth_generated_key_pair.cert_file_path
            ))
            .build()
            .start_without_healthcheck()
            .await;
        let graphql_endpoint = router.serv().url(router.graphql_path());

        let mut client_auth_buf = Vec::new();
        client_auth_buf.extend_from_slice(client_auth_generated_key_pair.cert_pem.as_bytes());
        client_auth_buf.extend_from_slice(client_auth_generated_key_pair.key_pem.as_bytes());
        let identity = reqwest::Identity::from_pem(&client_auth_buf)
            .expect("Failed to create identity from PEM file for mTLS test");

        let client = reqwest::Client::builder()
            .add_root_certificate(
                reqwest::Certificate::from_pem(generated_key_pair.cert_pem.as_bytes())
                    .expect("Failed to create certificate from PEM"),
            )
            .use_rustls_tls()
            .identity(identity)
            .build()
            .expect("Failed to build reqwest client with custom TLS configuration");
        let resp = client
            .post(graphql_endpoint)
            .json(&json!({
                "query": "{ me { name } }"
            }))
            .send()
            .await
            .expect("Failed to send request to router with TLS");
        insta::assert_snapshot!(
            resp.text().await.expect("Failed to parse text response from router with TLS")
            , @r#"{"data":{"me":{"name":"Uri Goldshtein"}}}"#);
    }

    #[ntex::test]
    async fn mtls_router_two_certs() {
        init_rustls_crypto_provider();
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let generated_key_pair = generate_keypair().await;
        let client_auth_generated_key_pair_1 = generate_keypair().await;
        let client_auth_generated_key_pair_2 = generate_keypair().await;
        let ca_contains_both_pem = format!(
            "{}\n{}",
            client_auth_generated_key_pair_1.cert_pem, client_auth_generated_key_pair_2.cert_pem
        );
        let mut ca_file =
            NamedTempFile::new().expect("Failed to create temporary file for certificate");
        ca_file
            .write(ca_contains_both_pem.as_bytes())
            .expect("Failed to write combined certificate to temporary file");

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
                        client_auth:
                            cert_file: "{}"
                "#,
                generated_key_pair.key_file_path,
                generated_key_pair.cert_file_path,
                ca_file
                    .path()
                    .to_str()
                    .expect("Expected to have a path for the combined ca file")
            ))
            .build()
            .start_without_healthcheck()
            .await;
        let graphql_endpoint = router.serv().url(router.graphql_path());

        async fn test_with_client_auth_pair(
            graphql_endpoint: &str,
            generated_key_pair: &GeneratedKeyPair,
            client_auth_generated_key_pair: &GeneratedKeyPair,
        ) {
            let mut client_auth_buf = Vec::new();
            client_auth_buf.extend_from_slice(client_auth_generated_key_pair.cert_pem.as_bytes());
            client_auth_buf.extend_from_slice(client_auth_generated_key_pair.key_pem.as_bytes());
            let identity = reqwest::Identity::from_pem(&client_auth_buf)
                .expect("Failed to create identity from PEM file for mTLS test");

            let client = reqwest::Client::builder()
                .add_root_certificate(
                    reqwest::Certificate::from_pem(generated_key_pair.cert_pem.as_bytes())
                        .expect("Failed to create certificate from PEM"),
                )
                .use_rustls_tls()
                .identity(identity)
                .build()
                .expect("Failed to build reqwest client with custom TLS configuration");
            let resp = client
                .post(graphql_endpoint)
                .json(&json!({
                    "query": "{ me { name } }"
                }))
                .send()
                .await
                .expect("Failed to send request to router with TLS");

            assert_eq!(resp.status(), 200, "Expected 200 OK from router with mTLS");
            assert_eq!(
                resp.text()
                    .await
                    .expect("Failed to parse text response from router with TLS"),
                r#"{"data":{"me":{"name":"Uri Goldshtein"}}}"#,
                "Unexpected response body from router with mTLS"
            );
        }

        test_with_client_auth_pair(
            &graphql_endpoint,
            &generated_key_pair,
            &client_auth_generated_key_pair_1,
        )
        .await;

        test_with_client_auth_pair(
            &graphql_endpoint,
            &generated_key_pair,
            &client_auth_generated_key_pair_2,
        )
        .await;
    }
}
