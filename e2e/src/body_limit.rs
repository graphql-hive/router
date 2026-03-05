#[cfg(test)]
mod body_limit_e2e_tests {
    use crate::testkit::{ClientResponseExt, TestRouter};
    #[ntex::test]
    async fn should_return_payload_too_large_if_limit_exceeds_while_reading_the_stream() {
        let router = TestRouter::builder()
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                limits:
                    max_request_body_size: 1B
                "#,
            )
            .build()
            .start()
            .await;

        // we must use a stream to avoid ntex setting the content-type
        let stream = Box::pin(futures::stream::once(async move {
            Ok::<_, std::io::Error>(ntex::util::Bytes::from(r#"{"query":"{__typename}"}"#))
        }));

        let res = router
            .serv()
            .post(router.graphql_path())
            .header(http::header::CONTENT_TYPE, "application/json")
            .send_stream(stream)
            .await
            .expect("failed to send graphql request");

        assert_eq!(res.status(), ntex::http::StatusCode::PAYLOAD_TOO_LARGE);

        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
        {
          "errors": [
            {
              "message": "Request body exceeds the maximum allowed size while reading the stream",
              "extensions": {
                "code": "PAYLOAD_TOO_LARGE_BODY_STREAM"
              }
            }
          ]
        }
        "#);
    }

    #[ntex::test]
    async fn should_return_payload_too_large_if_content_length_header_exceeds_the_limit() {
        let router = TestRouter::builder()
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                limits:
                    max_request_body_size: 1B
                "#,
            )
            .build()
            .start()
            .await;

        // use send_body instead of send_json to avoid a race condition
        // where ntex's send_json may encounter a "Disconnected" error in CI
        // when the server closes the connection before reading the full body
        // and responding with 413
        let body = sonic_rs::to_vec(&sonic_rs::json!({
            "query": "{ __typename }",
        }))
        .unwrap();

        let res = router
            .serv()
            .post(router.graphql_path())
            .header(http::header::CONTENT_TYPE, "application/json")
            .send_body(body)
            .await
            .expect("failed to send graphql request");

        assert_eq!(res.status(), ntex::http::StatusCode::PAYLOAD_TOO_LARGE);

        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
        {
          "errors": [
            {
              "message": "Content-Length exceeds the maximum allowed size: 1",
              "extensions": {
                "code": "PAYLOAD_TOO_LARGE_CONTENT_LENGTH"
              }
            }
          ]
        }
        "#);
    }
}
