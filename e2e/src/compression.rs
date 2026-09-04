#[cfg(test)]
mod compression_e2e_tests {
    use std::io::Write;

    use http::header::{ACCEPT_ENCODING, CONTENT_ENCODING, CONTENT_TYPE, VARY};
    use ntex::http::StatusCode;
    use sonic_rs::JsonValueTrait;

    use crate::testkit::{some_header_map, ClientResponseExt, Started, TestRouter, TestSubgraphs};

    /// ntex's test HTTP client transparently decompresses `gzip`/`deflate` response bodies
    /// (and strips nothing from the headers while doing it), which would hide exactly what
    /// we're trying to assert on here. `.no_decompress()` gets us the real wire bytes and
    /// headers, matching what an actual client would see before it decodes anything itself.
    async fn send_graphql_request_raw(
        router: &TestRouter<Started>,
        query: &str,
        headers: http::HeaderMap,
    ) -> ntex::client::ClientResponse {
        let mut req = router
            .serv()
            .post(router.graphql_path())
            .no_decompress()
            .header(CONTENT_TYPE, "application/json")
            .header(http::header::ACCEPT, "application/graphql-response+json");
        for (key, value) in headers.iter() {
            req = req.set_header(key, value);
        }
        req.send_json(&sonic_rs::json!({ "query": query, "variables": null }))
            .await
            .expect("failed to send graphql request")
    }

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
            .expect("failed to gunzip response body");
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
            .expect("failed to inflate response body");
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
            .expect("failed to un-brotli response body");
        out
    }

    fn zstd_compress(data: &[u8]) -> Vec<u8> {
        zstd::stream::encode_all(data, 3).expect("failed to zstd data")
    }

    fn zstd_decompress(data: &[u8]) -> Vec<u8> {
        zstd::stream::decode_all(data).expect("failed to un-zstd response body")
    }

    fn typename_query_body() -> Vec<u8> {
        sonic_rs::to_vec(&sonic_rs::json!({ "query": "{ __typename }" }))
            .expect("failed to serialize graphql request body")
    }

    fn assert_typename_response(body: &[u8]) {
        let value: sonic_rs::Value =
            sonic_rs::from_slice(body).expect("response body should be valid JSON");
        assert_eq!(
            value["data"]["__typename"].as_str(),
            Some("Query"),
            "unexpected response body: {}",
            String::from_utf8_lossy(body)
        );
    }

    #[ntex::test]
    async fn should_compress_response_with_gzip_when_client_accepts_it() {
        let router = TestRouter::builder()
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                traffic_shaping:
                    router:
                        compression:
                            response:
                                min_size: 1B
                "#,
            )
            .build()
            .start()
            .await;

        let res = send_graphql_request_raw(
            &router,
            "{ __typename }",
            some_header_map! { ACCEPT_ENCODING => "gzip" }.unwrap(),
        )
        .await;

        assert_eq!(res.status(), 200);
        assert_eq!(
            res.header(CONTENT_ENCODING.as_str()).map(|v| v.as_bytes()),
            Some(b"gzip".as_slice()),
            "expected a gzip-encoded response"
        );

        let raw_body = res.body().await.expect("failed to read response body");
        assert_typename_response(&gzip_decompress(&raw_body));
    }

    #[ntex::test]
    async fn should_compress_response_with_deflate_when_client_accepts_it() {
        let router = TestRouter::builder()
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                traffic_shaping:
                    router:
                        compression:
                            response:
                                min_size: 1B
                "#,
            )
            .build()
            .start()
            .await;

        let res = send_graphql_request_raw(
            &router,
            "{ __typename }",
            some_header_map! { ACCEPT_ENCODING => "deflate" }.unwrap(),
        )
        .await;

        assert_eq!(res.status(), 200);
        assert_eq!(
            res.header(CONTENT_ENCODING.as_str()).map(|v| v.as_bytes()),
            Some(b"deflate".as_slice()),
            "expected a deflate-encoded response"
        );

        let raw_body = res.body().await.expect("failed to read response body");
        assert_typename_response(&deflate_decompress(&raw_body));
    }

    #[ntex::test]
    async fn should_compress_response_with_brotli_when_client_accepts_it() {
        let router = TestRouter::builder()
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                traffic_shaping:
                    router:
                        compression:
                            response:
                                min_size: 1B
                "#,
            )
            .build()
            .start()
            .await;

        let res = send_graphql_request_raw(
            &router,
            "{ __typename }",
            some_header_map! { ACCEPT_ENCODING => "br" }.unwrap(),
        )
        .await;

        assert_eq!(res.status(), 200);
        assert_eq!(
            res.header(CONTENT_ENCODING.as_str()).map(|v| v.as_bytes()),
            Some(b"br".as_slice()),
            "expected a brotli-encoded response"
        );

        let raw_body = res.body().await.expect("failed to read response body");
        assert_typename_response(&brotli_decompress(&raw_body));
    }

    #[ntex::test]
    async fn should_compress_response_with_zstd_when_client_accepts_it() {
        let router = TestRouter::builder()
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                traffic_shaping:
                    router:
                        compression:
                            response:
                                min_size: 1B
                "#,
            )
            .build()
            .start()
            .await;

        let res = send_graphql_request_raw(
            &router,
            "{ __typename }",
            some_header_map! { ACCEPT_ENCODING => "zstd" }.unwrap(),
        )
        .await;

        assert_eq!(res.status(), 200);
        assert_eq!(
            res.header(CONTENT_ENCODING.as_str()).map(|v| v.as_bytes()),
            Some(b"zstd".as_slice()),
            "expected a zstd-encoded response"
        );

        let raw_body = res.body().await.expect("failed to read response body");
        assert_typename_response(&zstd_decompress(&raw_body));
    }

    #[ntex::test]
    async fn should_not_compress_response_when_client_does_not_accept_any_configured_algorithm() {
        let router = TestRouter::builder()
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

        // "compress" (old Unix LZW) is a registered content-coding token that's never actually
        // implemented anywhere, so it's not in the default `algorithms` allow-list. The router
        // must fall back to an uncompressed response instead of erroring.
        let res = send_graphql_request_raw(
            &router,
            "{ __typename }",
            some_header_map! { ACCEPT_ENCODING => "compress" }.unwrap(),
        )
        .await;

        assert_eq!(res.status(), 200);
        assert_eq!(
            res.header(CONTENT_ENCODING.as_str()),
            None,
            "response must not be encoded when the client only accepts an unsupported algorithm"
        );

        let raw_body = res.body().await.expect("failed to read response body");
        assert_typename_response(&raw_body);
    }

    #[ntex::test]
    async fn should_skip_compression_for_responses_smaller_than_min_size() {
        let router = TestRouter::builder()
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

        // default min_size is 1KiB; `{ __typename }` produces a response far smaller than that.
        let res = send_graphql_request_raw(
            &router,
            "{ __typename }",
            some_header_map! { ACCEPT_ENCODING => "gzip" }.unwrap(),
        )
        .await;

        assert_eq!(res.status(), 200);
        assert_eq!(
            res.header(CONTENT_ENCODING.as_str()),
            None,
            "tiny responses should be left uncompressed under the default min_size"
        );
    }

    #[ntex::test]
    async fn should_compress_responses_at_or_above_min_size() {
        let router = TestRouter::builder()
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                traffic_shaping:
                    router:
                        compression:
                            response:
                                min_size: 1B
                "#,
            )
            .build()
            .start()
            .await;

        let res = send_graphql_request_raw(
            &router,
            "{ __typename }",
            some_header_map! { ACCEPT_ENCODING => "gzip" }.unwrap(),
        )
        .await;

        assert_eq!(res.status(), 200);
        assert_eq!(
            res.header(CONTENT_ENCODING.as_str()).map(|v| v.as_bytes()),
            Some(b"gzip".as_slice()),
            "with min_size lowered to 1B, even a tiny response should be compressed"
        );
    }

    #[ntex::test]
    async fn should_not_compress_response_when_response_compression_disabled() {
        let router = TestRouter::builder()
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                traffic_shaping:
                    router:
                        compression:
                            response:
                                enabled: false
                                min_size: 1B
                "#,
            )
            .build()
            .start()
            .await;

        let res = send_graphql_request_raw(
            &router,
            "{ __typename }",
            some_header_map! { ACCEPT_ENCODING => "gzip" }.unwrap(),
        )
        .await;

        assert_eq!(res.status(), 200);
        assert_eq!(
            res.header(CONTENT_ENCODING.as_str()),
            None,
            "response compression must be entirely off when response.enabled is false"
        );
    }

    #[ntex::test]
    async fn should_not_compress_streaming_subscription_response() {
        let subgraphs = TestSubgraphs::builder()
            .with_http_streaming_subscriptions_protocol(
                subgraphs::HTTPStreamingSubscriptionProtocol::SseOnly,
            )
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
                subscriptions:
                    enabled: true
                traffic_shaping:
                    router:
                        compression:
                            response:
                                min_size: 1B
                "#,
            )
            .build()
            .start()
            .await;

        let res = send_graphql_request_raw(
            &router,
            r#"
            subscription {
                reviewAdded(intervalInMs: 0) {
                    product {
                        upc
                    }
                }
            }
            "#,
            some_header_map! {
                ACCEPT_ENCODING => "gzip",
                http::header::ACCEPT => "text/event-stream"
            }
            .unwrap(),
        )
        .await;

        assert_eq!(res.status(), 200);
        assert_eq!(
            res.header(CONTENT_ENCODING.as_str()),
            None,
            "streaming (SSE/multipart) responses must never be compressed, \
             since compressors buffer output and would delay live event delivery"
        );
    }

    #[ntex::test]
    async fn should_accept_gzip_compressed_request_body() {
        let router = TestRouter::builder()
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

        let compressed = gzip_compress(&typename_query_body());

        let res = router
            .serv()
            .post(router.graphql_path())
            .header(CONTENT_TYPE, "application/json")
            .header(CONTENT_ENCODING, "gzip")
            .send_body(compressed)
            .await
            .expect("failed to send gzip-compressed request");

        assert_eq!(res.status(), StatusCode::OK);
        let body = res.body().await.expect("failed to read response body");
        assert_typename_response(&body);
    }

    #[ntex::test]
    async fn should_accept_deflate_compressed_request_body() {
        let router = TestRouter::builder()
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

        let compressed = deflate_compress(&typename_query_body());

        let res = router
            .serv()
            .post(router.graphql_path())
            .header(CONTENT_TYPE, "application/json")
            .header(CONTENT_ENCODING, "deflate")
            .send_body(compressed)
            .await
            .expect("failed to send deflate-compressed request");

        assert_eq!(res.status(), StatusCode::OK);
        let body = res.body().await.expect("failed to read response body");
        assert_typename_response(&body);
    }

    #[ntex::test]
    async fn should_accept_brotli_compressed_request_body() {
        let router = TestRouter::builder()
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

        let compressed = brotli_compress(&typename_query_body());

        let res = router
            .serv()
            .post(router.graphql_path())
            .header(CONTENT_TYPE, "application/json")
            .header(CONTENT_ENCODING, "br")
            .send_body(compressed)
            .await
            .expect("failed to send brotli-compressed request");

        assert_eq!(res.status(), StatusCode::OK);
        let body = res.body().await.expect("failed to read response body");
        assert_typename_response(&body);
    }

    #[ntex::test]
    async fn should_accept_zstd_compressed_request_body() {
        let router = TestRouter::builder()
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

        let compressed = zstd_compress(&typename_query_body());

        let res = router
            .serv()
            .post(router.graphql_path())
            .header(CONTENT_TYPE, "application/json")
            .header(CONTENT_ENCODING, "zstd")
            .send_body(compressed)
            .await
            .expect("failed to send zstd-compressed request");

        assert_eq!(res.status(), StatusCode::OK);
        let body = res.body().await.expect("failed to read response body");
        assert_typename_response(&body);
    }

    #[ntex::test]
    async fn should_reject_unsupported_content_encoding_with_415() {
        let router = TestRouter::builder()
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

        // "compress" (old Unix LZW) is not in the default `algorithms` allow-list.
        let res = router
            .serv()
            .post(router.graphql_path())
            .header(CONTENT_TYPE, "application/json")
            .header(CONTENT_ENCODING, "compress")
            .send_body(typename_query_body())
            .await
            .expect("request should get an HTTP response, not a transport error");

        assert_eq!(res.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    }

    #[ntex::test]
    async fn should_reject_multi_valued_content_encoding_with_415() {
        let router = TestRouter::builder()
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

        // stacked codings (e.g. "gzip, br") are explicitly out of scope; the router should
        // reject cleanly rather than attempt a single-pass decode of double-encoded bytes.
        let compressed = gzip_compress(&typename_query_body());

        let res = router
            .serv()
            .post(router.graphql_path())
            .header(CONTENT_TYPE, "application/json")
            .header(CONTENT_ENCODING, "gzip, br")
            .send_body(compressed)
            .await
            .expect("request should get an HTTP response, not a transport error");

        assert_eq!(res.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    }

    #[ntex::test]
    async fn should_reject_compressed_request_when_decompression_disabled() {
        let router = TestRouter::builder()
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                traffic_shaping:
                    router:
                        compression:
                            request:
                                enabled: false
                "#,
            )
            .build()
            .start()
            .await;

        let compressed = gzip_compress(&typename_query_body());

        let res = router
            .serv()
            .post(router.graphql_path())
            .header(CONTENT_TYPE, "application/json")
            .header(CONTENT_ENCODING, "gzip")
            .send_body(compressed)
            .await
            .expect("request should get an HTTP response, not a transport error");

        assert_eq!(res.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    }

    #[ntex::test]
    async fn should_reject_oversized_decompressed_body_with_413() {
        let router = TestRouter::builder()
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                limits:
                    max_request_body_size: 100B
                "#,
            )
            .build()
            .start()
            .await;

        // Highly compressible payload: tiny on the wire, but decompresses past the
        // configured 100B limit. The size guard must apply to the inflated bytes.
        let inflated = vec![b'a'; 10_000];
        let compressed = gzip_compress(&inflated);

        let res = router
            .serv()
            .post(router.graphql_path())
            .header(CONTENT_TYPE, "application/json")
            .header(CONTENT_ENCODING, "gzip")
            .send_body(compressed)
            .await;

        match res {
            Ok(res) => assert_eq!(res.status(), StatusCode::PAYLOAD_TOO_LARGE),
            Err(e) => panic!("expected an HTTP 413 response, got a transport error: {e:?}"),
        }
    }

    // Confirms something similar to Apollo Router's CVE-2024-28101: the request body size
    // limit was only enforced *after* the compressed body had been fully decompressed into
    // memory, so a small, highly compressible payload could exhaust memory before the check
    // ever ran
    #[ntex::test]
    async fn should_reject_single_chunk_decompression_bomb_promptly() {
        let router = TestRouter::builder()
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                limits:
                    max_request_body_size: 50KiB
                "#,
            )
            .build()
            .start()
            .await;

        // ~25MB of a single repeated byte compresses to ~24KB with gzip - comfortably under
        // the 50KiB limit on the wire, but the decompressed size (25MB) is nowhere close.
        let inflated = vec![b'a'; 25_000_000];
        let compressed = gzip_compress(&inflated);
        assert!(
            compressed.len() < 50 * 1024,
            "test payload must compress to under the 50KiB limit for this test to be meaningful, got {} bytes",
            compressed.len()
        );

        let send = router
            .serv()
            .post(router.graphql_path())
            .header(CONTENT_TYPE, "application/json")
            .header(CONTENT_ENCODING, "gzip")
            .send_body(compressed);

        let res = tokio::time::timeout(std::time::Duration::from_secs(10), send)
            .await
            .expect("router did not respond within 10s - something is going wrong");

        match res {
            Ok(res) => {
                assert_eq!(res.status(), StatusCode::PAYLOAD_TOO_LARGE);
                let body: sonic_rs::Value = res.json_body().await;

                // PAYLOAD_TOO_LARGE_BODY_STREAM is set when we really hit the limit while
                // reading the body, and not based on content-length
                assert_eq!(
                    body["errors"][0]["extensions"]["code"].as_str(),
                    Some("PAYLOAD_TOO_LARGE_BODY_STREAM"),
                    "expected the decompressed-size guard to reject this, not the \
                     Content-Length fast-path, got: {body:?}"
                );
            }
            Err(e) => panic!("expected an HTTP 413 response, got a transport error: {e:?}"),
        }
    }

    #[ntex::test]
    async fn should_reject_malformed_body_for_each_algorithm_with_400() {
        let router = TestRouter::builder()
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

        let garbage = b"this is plain text, not compressed data of any kind: 1234567890".to_vec();

        for algorithm in ["gzip", "deflate", "br", "zstd"] {
            let res = router
                .serv()
                .post(router.graphql_path())
                .header(CONTENT_TYPE, "application/json")
                .header(CONTENT_ENCODING, algorithm)
                .send_body(garbage.clone())
                .await
                .unwrap_or_else(|e| {
                    panic!("[{algorithm}] expected an HTTP response, got a transport error: {e:?}")
                });

            assert_eq!(
                res.status(),
                StatusCode::BAD_REQUEST,
                "[{algorithm}] expected 400 for a body that doesn't match the claimed encoding"
            );
        }
    }

    #[ntex::test]
    async fn should_never_propagate_content_encoding_to_subgraphs() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                headers:
                    all:
                        request:
                            - propagate:
                                matching: ".*"
                "#,
            )
            .build()
            .start()
            .await;

        let compressed = gzip_compress(
            &sonic_rs::to_vec(&sonic_rs::json!({ "query": "{ users { id } }" })).unwrap(),
        );

        let res = router
            .serv()
            .post(router.graphql_path())
            .header(CONTENT_TYPE, "application/json")
            .header(CONTENT_ENCODING, "gzip")
            .send_body(compressed)
            .await
            .expect("failed to send gzip-compressed request");

        assert_eq!(res.status(), StatusCode::OK);

        let subgraph_requests = subgraphs
            .get_requests_log("accounts")
            .expect("expected requests sent to accounts subgraph");
        assert_eq!(
            subgraph_requests.len(),
            1,
            "expected 1 request to accounts subgraph"
        );

        let subgraph_request = &subgraph_requests[0];
        assert!(
            subgraph_request.headers.get("content-encoding").is_none(),
            "content-encoding must never be forwarded to subgraphs, even under a \".*\" propagate rule"
        );

        // the subgraph must have received the already-decompressed, valid JSON body
        let subgraph_body = subgraph_request
            .body
            .as_ref()
            .expect("expected a body to have been sent to the subgraph");
        sonic_rs::from_slice::<sonic_rs::Value>(subgraph_body)
            .expect("subgraph should have received valid, decompressed JSON");
    }

    // test the `>=` semantics of `min_size`
    #[ntex::test]
    async fn should_treat_min_size_as_an_inclusive_lower_bound() {
        let probe_router = TestRouter::builder()
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
        let probe_res =
            send_graphql_request_raw(&probe_router, "{ __typename }", http::HeaderMap::new()).await;
        let body_len = probe_res
            .body()
            .await
            .expect("failed to read probe response body")
            .len();

        for (min_size, should_compress) in [
            (body_len + 1, false),
            (body_len, true),
            (body_len.saturating_sub(1).max(1), true),
        ] {
            let router = TestRouter::builder()
                .inline_config(format!(
                    r#"
                    supergraph:
                        source: file
                        path: supergraph.graphql
                    traffic_shaping:
                        router:
                            compression:
                                response:
                                    min_size: {min_size}B
                    "#
                ))
                .build()
                .start()
                .await;

            let res = send_graphql_request_raw(
                &router,
                "{ __typename }",
                some_header_map! { ACCEPT_ENCODING => "gzip" }.unwrap(),
            )
            .await;

            let is_compressed = res.header(CONTENT_ENCODING.as_str()).is_some();
            assert_eq!(
                is_compressed, should_compress,
                "min_size={min_size}, response body is {body_len} bytes uncompressed"
            );
        }
    }

    fn vary_contains(res: &ntex::client::ClientResponse, token: &str) -> bool {
        res.header(VARY.as_str())
            .and_then(|v| v.to_str().ok())
            .map(|v| v.split(',').any(|t| t.trim().eq_ignore_ascii_case(token)))
            .unwrap_or(false)
    }

    #[ntex::test]
    async fn should_set_vary_accept_encoding_when_compression_is_negotiated() {
        let router = TestRouter::builder()
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                traffic_shaping:
                    router:
                        compression:
                            response:
                                min_size: 1B
                "#,
            )
            .build()
            .start()
            .await;

        let res = send_graphql_request_raw(
            &router,
            "{ __typename }",
            some_header_map! { ACCEPT_ENCODING => "gzip" }.unwrap(),
        )
        .await;

        assert!(
            res.header(CONTENT_ENCODING.as_str()).is_some(),
            "sanity check: response should actually be compressed here"
        );
        assert!(
            vary_contains(&res, "accept-encoding"),
            "expected Vary to include accept-encoding, got: {:?}",
            res.header(VARY.as_str())
        );
    }

    #[ntex::test]
    async fn should_set_vary_accept_encoding_even_when_response_is_not_compressed() {
        let router = TestRouter::builder()
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

        let res = send_graphql_request_raw(&router, "{ __typename }", http::HeaderMap::new()).await;

        assert!(res.header(CONTENT_ENCODING.as_str()).is_none());
        assert!(
            vary_contains(&res, "accept-encoding"),
            "expected Vary to include accept-encoding even for an uncompressed response, got: {:?}",
            res.header(VARY.as_str())
        );
    }

    #[ntex::test]
    async fn should_append_to_existing_vary_header_instead_of_overwriting() {
        let router = TestRouter::builder()
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                traffic_shaping:
                    router:
                        compression:
                            response:
                                min_size: 1B
                cors:
                    enabled: true
                    policies:
                        - origins: ["https://example.com"]
                "#,
            )
            .build()
            .start()
            .await;

        let res = send_graphql_request_raw(
            &router,
            "{ __typename }",
            some_header_map! {
                ACCEPT_ENCODING => "gzip",
                http::header::ORIGIN => "https://example.com"
            }
            .unwrap(),
        )
        .await;

        assert!(
            vary_contains(&res, "origin"),
            "expected CORS's own Vary: Origin to survive, got: {:?}",
            res.header(VARY.as_str())
        );
        assert!(
            vary_contains(&res, "accept-encoding"),
            "expected accept-encoding to be appended, not to replace CORS's Vary, got: {:?}",
            res.header(VARY.as_str())
        );
    }

    #[ntex::test]
    async fn should_prefer_higher_quality_algorithm_via_accept_encoding_q_values() {
        let router = TestRouter::builder()
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                traffic_shaping:
                    router:
                        compression:
                            response:
                                min_size: 1B
                "#,
            )
            .build()
            .start()
            .await;

        // gzip is listed first (and is earlier in the default `algorithms` preference order),
        // but br has the higher q-value and must win.
        let res = send_graphql_request_raw(
            &router,
            "{ __typename }",
            some_header_map! { ACCEPT_ENCODING => "gzip;q=0.1, br;q=0.9" }.unwrap(),
        )
        .await;

        assert_eq!(
            res.header(CONTENT_ENCODING.as_str()).map(|v| v.as_bytes()),
            Some(b"br".as_slice()),
            "the higher q-value (br) should win over gzip, despite gzip being listed first"
        );
    }

    #[ntex::test]
    async fn should_compress_with_wildcard_accept_encoding() {
        let router = TestRouter::builder()
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                traffic_shaping:
                    router:
                        compression:
                            response:
                                min_size: 1B
                "#,
            )
            .build()
            .start()
            .await;

        let res = send_graphql_request_raw(
            &router,
            "{ __typename }",
            some_header_map! { ACCEPT_ENCODING => "*" }.unwrap(),
        )
        .await;

        // default `algorithms` preference order is [gzip, zstd, br, deflate]
        assert_eq!(
            res.header(CONTENT_ENCODING.as_str()).map(|v| v.as_bytes()),
            Some(b"gzip".as_slice()),
            "a bare wildcard should pick the router's own top preference"
        );
    }

    // Regression test: per RFC 9110 §12.5.3, `*` only matches content-codings that aren't
    // explicitly named elsewhere in the field. `gzip;q=0` explicitly marks gzip as
    // unacceptable, and `*` must not override that even though gzip is the router's own top
    // preference - it should fall through to the next preferred algorithm the client didn't
    // explicitly exclude.
    #[ntex::test]
    async fn should_not_let_wildcard_override_an_explicit_q0_exclusion() {
        let router = TestRouter::builder()
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                traffic_shaping:
                    router:
                        compression:
                            response:
                                min_size: 1B
                "#,
            )
            .build()
            .start()
            .await;

        let res = send_graphql_request_raw(
            &router,
            "{ __typename }",
            some_header_map! { ACCEPT_ENCODING => "gzip;q=0, *" }.unwrap(),
        )
        .await;

        // default `algorithms` preference order is [gzip, zstd, br, deflate] - gzip is
        // explicitly excluded, so the wildcard must resolve to zstd (the next preference).
        assert_eq!(
            res.header(CONTENT_ENCODING.as_str()).map(|v| v.as_bytes()),
            Some(b"zstd".as_slice()),
            "gzip;q=0 must stay unacceptable even though * is also present"
        );
    }

    // Regression test: if every configured algorithm is explicitly excluded with q=0, the
    // wildcard must not match any of them, and the response should be left uncompressed.
    #[ntex::test]
    async fn should_not_compress_when_wildcard_cannot_satisfy_any_explicitly_excluded_algorithm() {
        let router = TestRouter::builder()
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                traffic_shaping:
                    router:
                        compression:
                            response:
                                min_size: 1B
                                algorithms:
                                    - kind: gzip
                                    - kind: deflate
                "#,
            )
            .build()
            .start()
            .await;

        let res = send_graphql_request_raw(
            &router,
            "{ __typename }",
            some_header_map! { ACCEPT_ENCODING => "gzip;q=0, deflate;q=0, *" }.unwrap(),
        )
        .await;

        assert_eq!(
            res.header(CONTENT_ENCODING.as_str()),
            None,
            "every configured algorithm was explicitly excluded, so * has nothing left to match"
        );
    }

    #[ntex::test]
    async fn should_negotiate_response_encoding_case_insensitively() {
        let router = TestRouter::builder()
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                traffic_shaping:
                    router:
                        compression:
                            response:
                                min_size: 1B
                "#,
            )
            .build()
            .start()
            .await;

        let res = send_graphql_request_raw(
            &router,
            "{ __typename }",
            some_header_map! { ACCEPT_ENCODING => "GZIP" }.unwrap(),
        )
        .await;

        assert_eq!(
            res.header(CONTENT_ENCODING.as_str()).map(|v| v.as_bytes()),
            Some(b"gzip".as_slice()),
            "Accept-Encoding token matching must be case-insensitive"
        );
    }

    #[ntex::test]
    async fn should_decompress_request_encoding_case_insensitively() {
        let router = TestRouter::builder()
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

        let compressed = gzip_compress(&typename_query_body());

        let res = router
            .serv()
            .post(router.graphql_path())
            .header(CONTENT_TYPE, "application/json")
            .header(CONTENT_ENCODING, "GZIP")
            .send_body(compressed)
            .await
            .expect("failed to send gzip-compressed request");

        assert_eq!(res.status(), StatusCode::OK);
        let body = res.body().await.expect("failed to read response body");
        assert_typename_response(&body);
    }

    #[ntex::test]
    async fn should_not_compress_when_accept_encoding_header_is_empty() {
        let router = TestRouter::builder()
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                traffic_shaping:
                    router:
                        compression:
                            response:
                                min_size: 1B
                "#,
            )
            .build()
            .start()
            .await;

        let res = send_graphql_request_raw(
            &router,
            "{ __typename }",
            some_header_map! { ACCEPT_ENCODING => "" }.unwrap(),
        )
        .await;

        assert_eq!(res.status(), 200);
        assert!(res.header(CONTENT_ENCODING.as_str()).is_none());
    }
}
