#[cfg(test)]
mod body_limit_e2e_tests {
    use crate::testkit::{ClientResponseExt, TestRouterBuilder};
    #[ntex::test]
    async fn should_return_payload_too_large_if_limit_exceeds_while_reading_the_stream() {
        let router = TestRouterBuilder::new()
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
        let router = TestRouterBuilder::new()
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

        // ntex will set the content-type and it will exceede the 1B
        let res = router
            .send_graphql_request("{ __typename }", None, None)
            .await;

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
