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
    async fn should_return_431_if_many_small_headers_sum_exceeds_the_limit() {
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

        let mut req = router
            .serv()
            .post(router.graphql_path())
            .header(http::header::CONTENT_TYPE, "application/json");

        // no single header comes close to the 1KiB limit; only their sum exceeds it
        for i in 0..20 {
            req = req.header(format!("x-test-header-{i}"), "a".repeat(40));
        }

        let res = req
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

    // a GET query travels in the request line, which ntex limits to 16KiB by default
    #[ntex::test]
    async fn should_accept_a_get_request_line_above_16kib() {
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

        let query = format!("{{{}}}", "__typename%20".repeat(2000));
        let url = format!("{}?query={query}", router.graphql_path());
        assert!(url.len() > 16 * 1024, "the request line must exceed 16KiB");

        let res = router
            .serv()
            .get(url.as_str())
            .header(http::header::ACCEPT, "application/json")
            .send()
            .await
            .expect("failed to send graphql request");

        assert_eq!(res.status(), ntex::http::StatusCode::OK);
        let body = res.string_body().await;
        assert!(body.contains(r#""__typename":"Query""#), "{body}");
    }
}
