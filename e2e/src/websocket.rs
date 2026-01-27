#[cfg(test)]
mod websocket_e2e_tests {
    use futures::{future, Stream};
    use std::sync::Arc;

    use insta::assert_snapshot;
    use ntex::{http, util::Bytes, web::test};
    use reqwest::StatusCode;
    use sonic_rs::json;
    use subgraphs::InterceptedResponse;

    use crate::testkit::{
        init_graphql_request, init_router_from_config_file, init_router_from_config_inline,
        test_router, wait_for_readiness, SubgraphsServer, TestRouterConf,
    };

    fn get_content_type_header(res: &ntex::web::WebResponse) -> String {
        res.headers()
            .get(ntex::http::header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap()
            .to_string()
    }

    #[ntex::test]
    async fn query_over_websocket() {
        let _subgraphs_server = SubgraphsServer::start().await;

        let router = test_router(TestRouterConf::inline(&format!(
            r#"
            supergraph:
                source: file
                path: supergraph.graphql
            "#
        )))
        .await
        .unwrap();

        let mut res = router
            .graphql_request()
            .send_json(&json!({
              "query": "{ topProducts { name }}",
            }))
            .await
            .unwrap();

        assert!(
            res.status() == StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "Expected 415 Unsupported Media Type"
        );

        let body = res.json::<sonic_rs::Value>();

        assert_snapshot!(body, @"");
    }
}
