#[cfg(test)]
mod header_limit_e2e_tests {
    use crate::testkit::{ClientResponseExt, TestRouter};

    #[ntex::test]
    async fn should_return_431_if_request_headers_exceed_the_limit() {
        let router = TestRouter::builder()
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                limits:
                    max_request_header_size: 1KiB
                "#,
            )
            .build()
            .start()
            .await;

        let res = router
            .serv()
            .post(router.graphql_path())
            .header(http::header::CONTENT_TYPE, "application/json")
            .header(http::header::COOKIE, "a".repeat(2048))
            .send_body(r#"{"query":"{ __typename }"}"#)
            .await
            .expect("failed to send graphql request");

        assert_eq!(
            res.status(),
            ntex::http::StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE
        );

        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
        {
          "errors": [
            {
              "message": "Request headers exceed the maximum allowed size",
              "extensions": {
                "code": "REQUEST_HEADER_FIELDS_TOO_LARGE"
              }
            }
          ]
        }
        "#);
    }

    #[ntex::test]
    async fn should_accept_request_headers_within_the_limit() {
        let router = TestRouter::builder()
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                limits:
                    max_request_header_size: 8KiB
                "#,
            )
            .build()
            .start()
            .await;

        let res = router
            .serv()
            .post(router.graphql_path())
            .header(http::header::CONTENT_TYPE, "application/json")
            .header(http::header::COOKIE, "a".repeat(1024))
            .send_body(r#"{"query":"{ __typename }"}"#)
            .await
            .expect("failed to send graphql request");

        assert_eq!(res.status(), ntex::http::StatusCode::OK);
    }
}
