#[cfg(test)]
mod subgraph_compression_e2e_tests {
    use std::io::Write;

    use bytes::Bytes;
    use http::{header::CONTENT_ENCODING, StatusCode};
    use sonic_rs::{JsonContainerTrait, JsonValueTrait};

    use crate::testkit::{ClientResponseExt, ResponseLike, TestRouter, TestSubgraphs};

    fn gzip_compress(data: &[u8]) -> Vec<u8> {
        use flate2::{write::GzEncoder, Compression};
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(data).expect("failed to gzip data");
        encoder.finish().expect("failed to finish gzip stream")
    }

    fn gzip_decompress(data: &[u8]) -> Vec<u8> {
        use flate2::read::GzDecoder;
        use std::io::Read;
        let mut decoder = GzDecoder::new(data);
        let mut out = Vec::new();
        decoder
            .read_to_end(&mut out)
            .expect("failed to gunzip request body");
        out
    }

    fn deflate_compress(data: &[u8]) -> Vec<u8> {
        use flate2::{write::ZlibEncoder, Compression};
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(data).expect("failed to deflate data");
        encoder.finish().expect("failed to finish deflate stream")
    }

    fn deflate_decompress(data: &[u8]) -> Vec<u8> {
        use flate2::read::ZlibDecoder;
        use std::io::Read;
        let mut decoder = ZlibDecoder::new(data);
        let mut out = Vec::new();
        decoder
            .read_to_end(&mut out)
            .expect("failed to inflate request body");
        out
    }

    fn brotli_compress(data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut writer = brotli::CompressorWriter::new(&mut out, 4096, 5, 22);
            writer.write_all(data).expect("failed to brotli data");
        }
        out
    }

    fn brotli_decompress(data: &[u8]) -> Vec<u8> {
        use std::io::Read;
        let mut decoder = brotli::Decompressor::new(data, 4096);
        let mut out = Vec::new();
        decoder
            .read_to_end(&mut out)
            .expect("failed to un-brotli request body");
        out
    }

    fn zstd_compress(data: &[u8]) -> Vec<u8> {
        zstd::stream::encode_all(data, 3).expect("failed to zstd data")
    }

    fn decompress(algorithm: &str, data: &[u8]) -> Vec<u8> {
        match algorithm {
            "gzip" => gzip_decompress(data),
            "deflate" => deflate_decompress(data),
            "br" => brotli_decompress(data),
            "zstd" => zstd::stream::decode_all(data).expect("failed to un-zstd request body"),
            other => panic!("unsupported algorithm in test: {other}"),
        }
    }

    fn compress(algorithm: &str, data: &[u8]) -> Vec<u8> {
        match algorithm {
            "gzip" => gzip_compress(data),
            "deflate" => deflate_compress(data),
            "br" => brotli_compress(data),
            "zstd" => zstd_compress(data),
            other => panic!("unsupported algorithm in test: {other}"),
        }
    }

    #[ntex::test]
    async fn should_compress_request_body_for_each_algorithm_when_enabled() {
        for algorithm in ["gzip", "deflate", "br", "zstd"] {
            let subgraphs = TestSubgraphs::builder().build().start().await;
            let router = TestRouter::builder()
                .with_subgraphs(&subgraphs)
                .inline_config(format!(
                    r#"
                    supergraph:
                        source: file
                        path: supergraph.graphql
                    traffic_shaping:
                        all:
                            compression:
                                request:
                                    enabled: true
                                    algorithm:
                                        kind: {algorithm}
                    "#
                ))
                .build()
                .start()
                .await;

            let res = router
                .send_graphql_request("{ users { id } }", None, None)
                .await;

            assert_eq!(
                res.status(),
                200,
                "[{algorithm}] expected a successful response"
            );

            let subgraph_requests = subgraphs
                .get_requests_log("accounts")
                .expect("expected requests sent to accounts subgraph");
            assert_eq!(
                subgraph_requests.len(),
                1,
                "[{algorithm}] expected 1 request to accounts subgraph"
            );

            let subgraph_request = &subgraph_requests[0];
            assert_eq!(
                subgraph_request
                    .headers
                    .get(CONTENT_ENCODING.as_str())
                    .map(|v| v.as_bytes()),
                Some(algorithm.as_bytes()),
                "[{algorithm}] expected the subgraph request to carry Content-Encoding"
            );

            let raw_body = subgraph_request
                .body
                .as_ref()
                .expect("expected a body to have been sent to the subgraph");
            let decompressed = decompress(algorithm, raw_body);
            let body: sonic_rs::Value = sonic_rs::from_slice(&decompressed).unwrap_or_else(|e| {
                panic!("[{algorithm}] subgraph should receive valid, decompressed JSON: {e}")
            });
            assert!(
                body["query"].as_str().is_some_and(|q| q.contains("users")),
                "[{algorithm}] decompressed subgraph request body should contain the forwarded query, got: {decompressed:?}"
            );
        }
    }

    #[ntex::test]
    async fn should_not_compress_request_body_by_default() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                "#,
            )
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request("{ users { id } }", None, None)
            .await;
        assert_eq!(res.status(), 200);

        let subgraph_requests = subgraphs
            .get_requests_log("accounts")
            .expect("expected requests sent to accounts subgraph");
        let subgraph_request = &subgraph_requests[0];

        assert!(
            subgraph_request
                .headers
                .get(CONTENT_ENCODING.as_str())
                .is_none(),
            "request compression must be off by default"
        );
    }

    // regardless of whether request compression is enabled, the router should always tell
    // subgraphs which encodings it can decompress in their responses.
    #[ntex::test]
    async fn should_always_advertise_accept_encoding_to_subgraphs() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                "#,
            )
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request("{ users { id } }", None, None)
            .await;
        assert_eq!(res.status(), 200);

        let subgraph_requests = subgraphs
            .get_requests_log("accounts")
            .expect("expected requests sent to accounts subgraph");
        let subgraph_request = &subgraph_requests[0];

        assert!(
            subgraph_request
                .headers
                .get(http::header::ACCEPT_ENCODING.as_str())
                .is_some(),
            "the router should always advertise Accept-Encoding to subgraphs"
        );
    }

    #[ntex::test]
    async fn should_apply_per_subgraph_compression_override() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
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
                            compression:
                                request:
                                    enabled: true
                                    algorithm:
                                        kind: gzip
                "#,
            )
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request("{ users { id } topProducts { upc } }", None, None)
            .await;
        assert_eq!(res.status(), 200);

        let accounts_requests = subgraphs
            .get_requests_log("accounts")
            .expect("expected requests to accounts");
        assert!(
            accounts_requests[0]
                .headers
                .get(CONTENT_ENCODING.as_str())
                .is_some(),
            "accounts has its own compression override and should compress"
        );

        if let Some(products_requests) = subgraphs.get_requests_log("products") {
            assert!(
                products_requests[0]
                    .headers
                    .get(CONTENT_ENCODING.as_str())
                    .is_none(),
                "products has no override and no `all` default, so it should not compress"
            );
        }
    }

    #[ntex::test]
    async fn should_decompress_subgraph_response_for_each_algorithm() {
        for algorithm in ["gzip", "deflate", "br", "zstd"] {
            let body = sonic_rs::to_vec(&sonic_rs::json!({
                "data": { "users": [{ "id": "compressed-user-1" }] }
            }))
            .unwrap();
            let compressed = compress(algorithm, &body);

            let subgraphs = TestSubgraphs::builder()
                .with_on_request(move |req| {
                    if req.path.contains("accounts") {
                        Some(ResponseLike {
                            status: StatusCode::OK,
                            headers: {
                                let mut h = http::HeaderMap::new();
                                h.insert(
                                    http::header::CONTENT_TYPE,
                                    "application/json".parse().unwrap(),
                                );
                                h.insert(CONTENT_ENCODING, algorithm.parse().unwrap());
                                h
                            },
                            body: Some(Bytes::from(compressed.clone())),
                        })
                    } else {
                        None
                    }
                })
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
                    "#,
                )
                .build()
                .start()
                .await;

            let res = router
                .send_graphql_request("{ users { id } }", None, None)
                .await;

            assert_eq!(
                res.status(),
                200,
                "[{algorithm}] expected a successful response"
            );
            let response_body = res.json_body().await;
            assert_eq!(
                response_body["data"]["users"][0]["id"].as_str(),
                Some("compressed-user-1"),
                "[{algorithm}] expected the router to transparently decompress the subgraph response, got: {response_body:?}"
            );
        }
    }

    #[ntex::test]
    async fn should_return_a_clean_error_for_malformed_compressed_subgraph_response() {
        for algorithm in ["gzip", "deflate", "br", "zstd"] {
            let garbage = b"this is not compressed data of any kind: 1234567890".to_vec();

            let subgraphs = TestSubgraphs::builder()
                .with_on_request(move |req| {
                    if req.path.contains("accounts") {
                        Some(ResponseLike {
                            status: StatusCode::OK,
                            headers: {
                                let mut h = http::HeaderMap::new();
                                h.insert(
                                    http::header::CONTENT_TYPE,
                                    "application/json".parse().unwrap(),
                                );
                                h.insert(CONTENT_ENCODING, algorithm.parse().unwrap());
                                h
                            },
                            body: Some(Bytes::from(garbage.clone())),
                        })
                    } else {
                        None
                    }
                })
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
                    "#,
                )
                .build()
                .start()
                .await;

            let res = router
                .send_graphql_request("{ users { id } }", None, None)
                .await;

            // the router should surface a normal GraphQL error (subgraph fetch failure), not
            // hang, panic, or crash the worker.
            let body = res.json_body().await;
            assert!(
                body["errors"].as_array().is_some_and(|e| !e.is_empty()),
                "[{algorithm}] expected a GraphQL error for a malformed compressed subgraph response, got: {body:?}"
            );
        }
    }

    // Content-Length must describe the bytes actually sent (the compressed body), not the
    // original JSON's length - a wrong length here would silently truncate or corrupt
    // every compressed subgraph request
    #[ntex::test]
    async fn should_set_content_length_to_the_compressed_body_size() {
        for algorithm in ["gzip", "deflate", "br", "zstd"] {
            let subgraphs = TestSubgraphs::builder().build().start().await;
            let router = TestRouter::builder()
                .with_subgraphs(&subgraphs)
                .inline_config(format!(
                    r#"
                    supergraph:
                        source: file
                        path: supergraph.graphql
                    traffic_shaping:
                        all:
                            compression:
                                request:
                                    enabled: true
                                    algorithm:
                                        kind: {algorithm}
                    "#
                ))
                .build()
                .start()
                .await;

            let res = router
                .send_graphql_request("{ users { id } }", None, None)
                .await;
            assert_eq!(
                res.status(),
                200,
                "[{algorithm}] expected a successful response"
            );

            let subgraph_requests = subgraphs
                .get_requests_log("accounts")
                .expect("expected requests sent to accounts subgraph");
            let subgraph_request = &subgraph_requests[0];

            let raw_body = subgraph_request
                .body
                .as_ref()
                .expect("expected a body to have been sent to the subgraph");
            let content_length: usize = subgraph_request
                .headers
                .get(http::header::CONTENT_LENGTH.as_str())
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse().ok())
                .unwrap_or_else(|| {
                    panic!(
                        "[{algorithm}] expected a Content-Length header on the compressed request"
                    )
                });

            assert_eq!(
                content_length,
                raw_body.len(),
                "[{algorithm}] Content-Length must match the actual (compressed) body size, \
                 not the original JSON size"
            );
        }
    }

    // A truncated compressed stream (valid header/magic bytes, cut off mid-stream)
    #[ntex::test]
    async fn should_return_a_clean_error_for_a_truncated_compressed_subgraph_response() {
        for algorithm in ["gzip", "deflate", "br", "zstd"] {
            let body = sonic_rs::to_vec(&sonic_rs::json!({
                "data": { "users": [{ "id": "1" }] }
            }))
            .unwrap();
            let compressed = compress(algorithm, &body);
            let truncated = compressed[..compressed.len() / 2].to_vec();

            let subgraphs = TestSubgraphs::builder()
                .with_on_request(move |req| {
                    if req.path.contains("accounts") {
                        Some(ResponseLike {
                            status: StatusCode::OK,
                            headers: {
                                let mut h = http::HeaderMap::new();
                                h.insert(
                                    http::header::CONTENT_TYPE,
                                    "application/json".parse().unwrap(),
                                );
                                h.insert(CONTENT_ENCODING, algorithm.parse().unwrap());
                                h
                            },
                            body: Some(Bytes::from(truncated.clone())),
                        })
                    } else {
                        None
                    }
                })
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
                    "#,
                )
                .build()
                .start()
                .await;

            let res = router
                .send_graphql_request("{ users { id } }", None, None)
                .await;

            let body = res.json_body().await;
            assert!(
                body["errors"].as_array().is_some_and(|e| !e.is_empty()),
                "[{algorithm}] expected a GraphQL error for a truncated compressed subgraph response, got: {body:?}"
            );
        }
    }

    // `Content-Encoding` names one algorithm, but the bytes are actually a different algorithm
    #[ntex::test]
    async fn should_return_a_clean_error_when_content_encoding_does_not_match_the_actual_bytes() {
        let mismatched_pairs = [
            ("zstd", "gzip"),
            ("gzip", "deflate"),
            ("deflate", "br"),
            ("br", "zstd"),
        ];

        for (declared, actual) in mismatched_pairs {
            let body = sonic_rs::to_vec(&sonic_rs::json!({
                "data": { "users": [{ "id": "1" }] }
            }))
            .unwrap();
            let compressed = compress(actual, &body);

            let subgraphs = TestSubgraphs::builder()
                .with_on_request(move |req| {
                    if req.path.contains("accounts") {
                        Some(ResponseLike {
                            status: StatusCode::OK,
                            headers: {
                                let mut h = http::HeaderMap::new();
                                h.insert(
                                    http::header::CONTENT_TYPE,
                                    "application/json".parse().unwrap(),
                                );
                                h.insert(CONTENT_ENCODING, declared.parse().unwrap());
                                h
                            },
                            body: Some(Bytes::from(compressed.clone())),
                        })
                    } else {
                        None
                    }
                })
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
                    "#,
                )
                .build()
                .start()
                .await;

            let res = router
                .send_graphql_request("{ users { id } }", None, None)
                .await;

            let body = res.json_body().await;
            assert!(
                body["errors"].as_array().is_some_and(|e| !e.is_empty()),
                "declared={declared} actual={actual}: expected a GraphQL error when \
                 Content-Encoding doesn't match the actual bytes, got: {body:?}"
            );
        }
    }

    // Make sure subgraph responses with errors are also handled the same way.
    #[ntex::test]
    async fn should_decompress_a_subgraph_response_that_itself_contains_a_graphql_error() {
        for algorithm in ["gzip", "deflate", "br", "zstd"] {
            let body = sonic_rs::to_vec(&sonic_rs::json!({
                "data": { "users": null },
                "errors": [{ "message": "compressed-subgraph-error-marker" }]
            }))
            .unwrap();
            let compressed = compress(algorithm, &body);

            let subgraphs = TestSubgraphs::builder()
                .with_on_request(move |req| {
                    if req.path.contains("accounts") {
                        Some(ResponseLike {
                            status: StatusCode::OK,
                            headers: {
                                let mut h = http::HeaderMap::new();
                                h.insert(
                                    http::header::CONTENT_TYPE,
                                    "application/json".parse().unwrap(),
                                );
                                h.insert(CONTENT_ENCODING, algorithm.parse().unwrap());
                                h
                            },
                            body: Some(Bytes::from(compressed.clone())),
                        })
                    } else {
                        None
                    }
                })
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
                    "#,
                )
                .build()
                .start()
                .await;

            let res = router
                .send_graphql_request("{ users { id } }", None, None)
                .await;

            let body = res.json_body().await;
            let error_codes: Vec<&str> = body["errors"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|e| e["extensions"]["code"].as_str())
                .collect();

            assert!(
                error_codes.contains(&"DOWNSTREAM_SERVICE_ERROR"),
                "[{algorithm}] expected the compressed subgraph error response to be decompressed \
                 and routed as a normal subgraph-reported error (DOWNSTREAM_SERVICE_ERROR), not a \
                 malformed-response failure, got: {body:?}"
            );
        }
    }
}
