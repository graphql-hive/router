#[cfg(test)]
mod max_tokens_e2e_tests {
    use crate::testkit::{init_graphql_request, wait_for_readiness, SubgraphsServer};
    use ntex::web::test;

    #[ntex::test]
    async fn does_not_reject_an_operation_below_token_limit() {
        let subgraphs_server = SubgraphsServer::start().await;
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

        let req = init_graphql_request("{ users { id } }", None);
        let resp = test::call_service(&app.app, req.to_request()).await;
        assert!(resp.status().is_success(), "Expected 200 OK");

        let subgraph_requests = subgraphs_server
            .get_subgraph_requests_log("accounts")
            .await
            .expect("expected requests sent to accounts subgraph");
        assert_eq!(
            subgraph_requests.len(),
            1,
            "expected 1 request to accounts subgraph"
        );
    }

    #[ntex::test]
    async fn rejects_an_operation_exceeding_token_limit() {
        let subgraphs_server = SubgraphsServer::start().await;
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

        let op = "query {
            a: users {
                id
            }
            b: users {
                id
            }
            c: users {
                id
            }
        }";
        let req = init_graphql_request(op, None);
        let resp = test::call_service(&app.app, req.to_request()).await;
        let body_bytes = test::read_body(resp).await;
        let body_str = std::str::from_utf8(&body_bytes).unwrap();
        assert!(body_str.contains("Token limit of 4 exceeded"));

        let subgraph_requests = subgraphs_server
            .get_subgraph_requests_log("accounts")
            .await
            .unwrap_or_default();
        assert_eq!(
            subgraph_requests.len(),
            0,
            "expected 0 requests to accounts subgraph"
        );
    }

    #[ntex::test]
    async fn rejects_an_operation_exceeding_token_limit_without_exposing_limits() {
        let subgraphs_server = SubgraphsServer::start().await;
        let app = crate::testkit::init_router_from_config_inline(
            r#"
            supergraph:
                source: file
                path: ./supergraph.graphql
            limits:
                max_tokens:
                    n: 4
                    expose_limits: false
            "#,
        )
        .await
        .unwrap();
        wait_for_readiness(&app.app).await;

        let op = "query {
            a: users {
                id
            }
            b: users {
                id
            }
            c: users {
                id
            }
        }";
        let req = init_graphql_request(op, None);
        let resp = test::call_service(&app.app, req.to_request()).await;
        let body_bytes = test::read_body(resp).await;
        let body_str = std::str::from_utf8(&body_bytes).unwrap();
        assert!(body_str.contains("Token limit exceeded"));

        let subgraph_requests = subgraphs_server
            .get_subgraph_requests_log("accounts")
            .await
            .unwrap_or_default();
        assert_eq!(
            subgraph_requests.len(),
            0,
            "expected 0 requests to accounts subgraph"
        );
    }
}
