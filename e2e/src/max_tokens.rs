#[cfg(test)]
mod max_tokens_e2e_tests {
    use crate::testkit::{init_graphql_request, wait_for_readiness};
    use ntex::web::test;

    #[ntex::test]
    async fn does_not_reject_an_operation_below_token_limit() {
        let app = crate::testkit::init_router_from_config_inline(
            r#"
            supergraph:
                source: file
                path: ./supergraph.graphql
            limits:
                max_tokens:
                    n: 100
            "#,
        )
        .await
        .unwrap();
        wait_for_readiness(&app.app).await;

        let req = init_graphql_request("{ a a a a a a a }", None);
        let resp = test::call_service(&app.app, req.to_request()).await;

        let body_bytes = test::read_body(resp).await;
        let body_str = std::str::from_utf8(&body_bytes).unwrap();
        assert!(!body_str.contains("exceeded"));
    }

    #[ntex::test]
    async fn rejects_an_operation_exceeding_token_limit() {
        let app = crate::testkit::init_router_from_config_inline(
            r#"
            supergraph:
                source: file
                path: ./supergraph.graphql
            limits:
                max_tokens:
                    n: 4
            "#,
        )
        .await
        .unwrap();
        wait_for_readiness(&app.app).await;

        let req = init_graphql_request("query { a a a a a a }", None);
        let resp = test::call_service(&app.app, req.to_request()).await;
        let body_bytes = test::read_body(resp).await;
        let body_str = std::str::from_utf8(&body_bytes).unwrap();
        assert!(body_str.contains("Token limit of 4 exceeded"));
    }

    #[ntex::test]
    async fn rejects_an_operation_exceeding_token_limit_without_exposing_limits() {
        let app = crate::testkit::init_router_from_config_inline(
            r#"
            supergraph:
                source: file
                path: ./supergraph.graphql
            limits:
                max_tokens:
                    n: 5
                    expose_limits: false
            "#,
        )
        .await
        .unwrap();
        wait_for_readiness(&app.app).await;

        let req = init_graphql_request("query { a a a a a a }", None);
        let resp = test::call_service(&app.app, req.to_request()).await;
        let body_bytes = test::read_body(resp).await;
        let body_str = std::str::from_utf8(&body_bytes).unwrap();
        assert!(body_str.contains("Token limit exceeded"));
    }
}
